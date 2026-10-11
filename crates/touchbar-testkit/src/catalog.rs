use touchbar_model::*;
#[derive(Default)]
pub struct CatalogBuilder {
    catalog: Catalog,
    plugin: Option<PluginId>,
}
pub fn catalog() -> CatalogBuilder {
    CatalogBuilder::default()
}
pub struct SetBuilder {
    pub spec: SetSpec,
}
pub struct ProfileBuilder {
    pub spec: ProfileSpec,
}
impl CatalogBuilder {
    pub fn plugin(mut self, name: &str) -> Self {
        let id: PluginId = name.parse().expect("plugin id");
        self.catalog.plugins.push(PluginSpec {
            id: id.clone(),
            revision: "test-1".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        });
        self.plugin = Some(id);
        self
    }
    pub fn set(mut self, name: &str, build: impl FnOnce(&mut SetBuilder)) -> Self {
        let plugin = self.plugin.as_ref().expect("plugin before set");
        let mut s = SetBuilder {
            spec: SetSpec::default_for(format!("{plugin}/{name}").parse().expect("set id")),
        };
        s.spec.logic = Some(plugin.clone());
        build(&mut s);
        self.catalog.sets.push(s.spec);
        self
    }
    pub fn profile(mut self, name: &str, build: impl FnOnce(&mut ProfileBuilder)) -> Self {
        let plugin = self.plugin.as_ref().expect("plugin before profile");
        let mut p = ProfileBuilder {
            spec: ProfileSpec::default_for(format!("{plugin}/{name}").parse().expect("profile id")),
        };
        build(&mut p);
        self.catalog.profiles.push(p.spec);
        self
    }
    pub fn binding(mut self, binding: Binding<LayerAction>) -> Self {
        self.catalog.bindings.push(binding);
        self
    }
    pub fn build(self) -> Catalog {
        let errors: Vec<_> = self
            .catalog
            .validate()
            .into_iter()
            .filter(|i| i.code != CatalogIssueCode::ProfileConflict)
            .collect();
        assert!(errors.is_empty(), "invalid fixture catalog: {errors:?}");
        self.catalog
    }
}
impl SetBuilder {
    pub fn widget(&mut self, widget: WidgetSpec) -> &mut Self {
        self.spec.widgets.push(widget);
        self
    }
    pub fn button(&mut self, id: &str) -> &mut Self {
        let mut widget = WidgetSpec::new(id.parse().expect("widget id"), WidgetKind::Button);
        widget.handlers.insert(ActionKind::Activated, id.into());
        self.widget(widget)
    }
    pub fn slider(&mut self, id: &str) -> &mut Self {
        let mut widget = WidgetSpec::new(
            id.parse().expect("widget id"),
            WidgetKind::Slider {
                minimum: 0.0,
                maximum: 1.0,
            },
        );
        widget.handlers.insert(ActionKind::ValueChanged, id.into());
        self.widget(widget)
    }
    pub fn key(&mut self, id: &str, key: SystemKey) -> &mut Self {
        self.widget(WidgetSpec::new(
            id.parse().expect("widget id"),
            WidgetKind::Key { key },
        ))
    }
    pub fn scrubber(&mut self, id: &str) -> &mut Self {
        let mut widget = WidgetSpec::new(id.parse().expect("widget id"), WidgetKind::Scrubber);
        widget.handlers.insert(ActionKind::Selected, id.into());
        widget
            .state_bindings
            .insert(StateBinding::Records, format!("{id}_items"));
        self.widget(widget)
    }
    pub fn panel(&mut self, panel: PanelSpec) -> &mut Self {
        self.spec.panels.push(panel);
        self
    }
    pub fn static_set(&mut self) -> &mut Self {
        self.spec.logic = None;
        self
    }
}
impl ProfileBuilder {
    pub fn default(&mut self) -> &mut Self {
        self.spec.default = true;
        self
    }
    pub fn slot(&mut self, id: &str, candidates: &[&str]) -> &mut Self {
        self.spec.slots.push(SlotSpec {
            id: id.parse().expect("slot id"),
            candidates: candidates
                .iter()
                .map(|s| s.parse().expect("set ref"))
                .collect(),
        });
        self
    }
    pub fn when(&mut self, condition: Condition) -> &mut Self {
        self.spec.when = Some(condition);
        self
    }
}
