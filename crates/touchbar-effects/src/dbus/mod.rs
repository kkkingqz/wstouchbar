pub mod codec;
use futures_lite::StreamExt;
use std::{collections::BTreeMap, sync::Arc};
use touchbar_model::*;
use zbus::{Connection, MatchRule, Message, MessageStream, message::Type, zvariant::OwnedValue};
fn failed(e: impl std::fmt::Display) -> EffectError {
    EffectError::Failed(e.to_string())
}
#[derive(Clone)]
pub struct Client {
    connections: Arc<[async_lock::Mutex<Option<Connection>>; 2]>,
    addresses: [Option<String>; 2],
}
impl Default for Client {
    fn default() -> Self {
        Self::with_addresses(None, None)
    }
}
impl Client {
    pub fn with_addresses(session: Option<String>, system: Option<String>) -> Self {
        Self {
            connections: Arc::new(std::array::from_fn(|_| async_lock::Mutex::new(None))),
            addresses: [session, system],
        }
    }
    async fn connection(&self, bus: Bus) -> Result<Connection, EffectError> {
        let index = match bus {
            Bus::Session => 0,
            Bus::System => 1,
        };
        let mut cached = self.connections[index].lock().await;
        if let Some(conn) = cached.as_ref().filter(|c| !c.is_closed()) {
            return Ok(conn.clone());
        }
        let conn = if let Some(address) = &self.addresses[index] {
            zbus::connection::Builder::address(address.as_str())
                .map_err(failed)?
                .build()
                .await
        } else {
            match bus {
                Bus::Session => Connection::session().await,
                Bus::System => Connection::system().await,
            }
        }
        .map_err(failed)?;
        *cached = Some(conn.clone());
        Ok(conn)
    }
    pub async fn call(&self, call: &DbusCall) -> Result<DbusValue, EffectError> {
        let body = codec::encode(&call.args)?;
        let conn = self.connection(call.bus).await?;
        let reply = conn
            .call_method(
                Some(call.destination.as_str()),
                call.path.as_str(),
                Some(call.interface.as_str()),
                call.method.as_str(),
                &body,
            )
            .await
            .map_err(failed)?;
        codec::decode_message(&reply)
    }
    pub async fn signals(&self, filter: DbusMatch) -> Result<Signals, EffectError> {
        let conn = self.connection(filter.bus).await?;
        let mut rule = MatchRule::builder().msg_type(Type::Signal);
        let mut owners = None;
        let mut owner = None;
        if let Some(sender) = filter.sender {
            let change = MatchRule::builder()
                .msg_type(Type::Signal)
                .sender("org.freedesktop.DBus")
                .map_err(failed)?
                .interface("org.freedesktop.DBus")
                .map_err(failed)?
                .member("NameOwnerChanged")
                .map_err(failed)?
                .arg(0, sender.clone())
                .map_err(failed)?
                .build();
            owners = Some(
                MessageStream::for_match_rule(change, &conn, Some(64))
                    .await
                    .map_err(failed)?,
            );
            let reply = conn
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetNameOwner",
                    &(sender,),
                )
                .await
                .map_err(|_| EffectError::Unavailable)?;
            let name = reply.body().deserialize::<String>().map_err(failed)?;
            rule = rule.sender(name.clone()).map_err(failed)?;
            owner = Some(name);
        }
        if let Some(path) = filter.path {
            rule = rule.path(path).map_err(failed)?;
        }
        if let Some(interface) = filter.interface {
            rule = rule.interface(interface).map_err(failed)?;
        }
        if let Some(member) = filter.member {
            rule = rule.member(member).map_err(failed)?;
        }
        let messages = MessageStream::for_match_rule(rule.build(), &conn, Some(64))
            .await
            .map_err(failed)?;
        Ok(Signals {
            messages,
            owners,
            owner,
            _connection: conn,
        })
    }
    pub async fn property(&self, spec: DbusProperty) -> Result<Property, EffectError> {
        let signals = self
            .signals(DbusMatch {
                bus: spec.bus,
                sender: Some(spec.destination.clone()),
                path: Some(spec.path.clone()),
                interface: Some("org.freedesktop.DBus.Properties".into()),
                member: Some("PropertiesChanged".into()),
            })
            .await?;
        let mut watch = Property {
            spec,
            signals,
            initial: None,
            last: None,
            position: None,
        };
        let (value, position) = watch.get().await?;
        watch.position = Some(position);
        watch.initial = Some(value);
        Ok(watch)
    }
}
pub struct Signals {
    messages: MessageStream,
    owners: Option<MessageStream>,
    owner: Option<String>,
    _connection: Connection,
}
// Owner loss takes priority over queued data and replies from the departed service.
async fn owner_lost(owners: &mut Option<MessageStream>, expected: Option<&str>) -> EffectError {
    if let Some(owners) = owners {
        loop {
            let message = match owners.next().await {
                Some(Ok(message)) => message,
                Some(Err(error)) => return failed(error),
                None => return EffectError::Unavailable,
            };
            let new = match message.body().deserialize::<(String, String, String)>() {
                Ok((_, _, new)) => new,
                Err(error) => return failed(error),
            };
            if Some(new.as_str()) != expected {
                return EffectError::Unavailable;
            }
        }
    }
    std::future::pending().await
}
impl Signals {
    async fn next_message(&mut self) -> Result<Message, EffectError> {
        let owner = async { Err(owner_lost(&mut self.owners, self.owner.as_deref()).await) };
        let data = async {
            let message = self
                .messages
                .next()
                .await
                .ok_or(EffectError::Unavailable)?
                .map_err(failed)?;
            if message.body().len() > 256 * 1024 {
                return Err(failed("D-Bus signal exceeds 256 KiB"));
            }
            Ok(message)
        };
        futures_lite::future::or(owner, data).await
    }
    pub async fn next(&mut self) -> Result<DbusValue, EffectError> {
        codec::decode_message(&self.next_message().await?)
    }
}
pub struct Property {
    spec: DbusProperty,
    signals: Signals,
    initial: Option<DbusValue>,
    last: Option<DbusValue>,
    position: Option<zbus::message::Sequence>,
}
impl Property {
    async fn get(&mut self) -> Result<(DbusValue, zbus::message::Sequence), EffectError> {
        // Snapshot and signals share a connection and the captured unique owner.
        let conn = &self.signals._connection;
        let owner = self.signals.owner.as_deref();
        let lost = async { Err(owner_lost(&mut self.signals.owners, owner).await) };
        let get = async {
            let reply = conn
                .call_method(
                    Some(owner.unwrap_or(self.spec.destination.as_str())),
                    self.spec.path.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &(self.spec.interface.as_str(), self.spec.property.as_str()),
                )
                .await
                .map_err(failed)?;
            let value = reply.body().deserialize::<OwnedValue>().map_err(failed)?;
            Ok((codec::property(&value)?, reply.recv_position()))
        };
        futures_lite::future::or(lost, get).await
    }
    pub async fn next(&mut self) -> Result<DbusValue, EffectError> {
        if let Some(error) = futures_lite::future::poll_once(owner_lost(
            &mut self.signals.owners,
            self.signals.owner.as_deref(),
        ))
        .await
        {
            return Err(error);
        }
        if let Some(value) = self.initial.take() {
            self.last = Some(value.clone());
            return Ok(value);
        }
        loop {
            let message = self.signals.next_message().await?;
            let position = message.recv_position();
            if self.position.is_some_and(|snapshot| position <= snapshot) {
                continue;
            }
            let (interface, values, invalidated) = message
                .body()
                .deserialize::<(String, BTreeMap<String, OwnedValue>, Vec<String>)>()
                .map_err(failed)?;
            if interface != self.spec.interface {
                continue;
            }
            let value = if let Some(value) = values.get(&self.spec.property) {
                self.position = Some(position);
                codec::property(value)?
            } else if invalidated.contains(&self.spec.property) {
                let (value, position) = self.get().await?;
                self.position = Some(position);
                value
            } else {
                continue;
            };
            if self.last.as_ref() != Some(&value) {
                self.last = Some(value.clone());
                return Ok(value);
            }
        }
    }
}
