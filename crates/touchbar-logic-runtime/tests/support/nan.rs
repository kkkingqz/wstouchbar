use touchbar_model::*;
// Normalize only actual floating point fields; text resembling IEEE tags remains text.
fn n32(v: &mut f32) {
    if v.is_nan() {
        *v = f32::NAN;
    }
}
fn n64(v: &mut f64) {
    if v.is_nan() {
        *v = f64::NAN;
    }
}
fn scalar(v: &mut Scalar) {
    if let Scalar::Number(v) = v {
        n64(v)
    }
}
fn point(p: &mut Point) {
    n32(&mut p.x);
    n32(&mut p.y);
}
fn rect(r: &mut Rect) {
    n32(&mut r.x);
    n32(&mut r.y);
    n32(&mut r.width);
    n32(&mut r.height);
}
fn paint(p: &mut CanvasPaint) {
    n32(&mut p.opacity);
    if let CanvasColor::Rgba {
        red,
        green,
        blue,
        alpha,
    } = &mut p.color
    {
        for v in [red, green, blue, alpha] {
            n32(v)
        }
    }
}
fn segments(s: &mut [CanvasPathSegment]) {
    for s in s {
        match s {
            CanvasPathSegment::MoveTo(p) | CanvasPathSegment::LineTo(p) => point(p),
            CanvasPathSegment::QuadraticTo { control, endpoint } => {
                point(control);
                point(endpoint)
            }
            CanvasPathSegment::CubicTo {
                control_one,
                control_two,
                endpoint,
            } => {
                point(control_one);
                point(control_two);
                point(endpoint)
            }
            CanvasPathSegment::Close => {}
        }
    }
}
fn state(patch: &mut StatePatch) {
    for (_, v) in patch {
        match v {
            StateValue::Scalar(v) => scalar(v),
            StateValue::Records(records) => {
                for record in records {
                    for v in record.values_mut() {
                        scalar(v)
                    }
                }
            }
            StateValue::Canvas(commands) => {
                for c in commands {
                    match c {
                        CanvasCommand::FillRect {
                            rect: r,
                            radius,
                            paint: p,
                        } => {
                            rect(r);
                            n32(radius);
                            paint(p)
                        }
                        CanvasCommand::Line {
                            start,
                            end,
                            width,
                            paint: p,
                        } => {
                            point(start);
                            point(end);
                            n32(width);
                            paint(p)
                        }
                        CanvasCommand::Polyline {
                            points,
                            width,
                            paint: p,
                        } => {
                            for p in points {
                                point(p)
                            }
                            n32(width);
                            paint(p)
                        }
                        CanvasCommand::FillCircle {
                            center,
                            radius,
                            paint: p,
                        } => {
                            point(center);
                            n32(radius);
                            paint(p)
                        }
                        CanvasCommand::Text {
                            rect: r,
                            size,
                            paint: p,
                            ..
                        } => {
                            rect(r);
                            n32(size);
                            paint(p)
                        }
                        CanvasCommand::FillLinearGradientRect {
                            rect: r,
                            radius,
                            gradient,
                        } => {
                            rect(r);
                            n32(radius);
                            point(&mut gradient.start);
                            point(&mut gradient.end);
                            paint(&mut gradient.start_color);
                            paint(&mut gradient.end_color)
                        }
                        CanvasCommand::StrokePath {
                            segments: s,
                            width,
                            paint: p,
                        } => {
                            segments(s);
                            n32(width);
                            paint(p)
                        }
                        CanvasCommand::FillPath {
                            segments: s,
                            paint: p,
                        } => {
                            segments(s);
                            paint(p)
                        }
                    }
                }
            }
        }
    }
}
pub fn normalize_update(mut u: LogicUpdate) -> serde_json::Value {
    state(&mut u.state);
    for r in &mut u.requests {
        match &mut r.effect {
            Effect::AudioSetVolume { level, .. } | Effect::DesktopBrightness { level } => {
                n64(level)
            }
            _ => {}
        }
    }
    serde_json::to_value(u).unwrap()
}
fn fact(f: &mut FactValue) {
    match f {
        FactValue::Number(v) => n64(v),
        FactValue::Records(rs) => {
            for r in rs {
                for v in r.values_mut() {
                    scalar(v)
                }
            }
        }
        _ => {}
    }
}
pub fn normalize_event(mut e: LogicEvent) -> serde_json::Value {
    match &mut e {
        LogicEvent::Action(e) => {
            if let Some(v) = &mut e.value {
                scalar(v)
            }
        }
        LogicEvent::ContextChanged(ctx) => n32(&mut ctx.slot_width),
        LogicEvent::SubscriptionData { data, .. } => match data {
            SubscriptionData::Audio(s) => n64(&mut s.volume),
            SubscriptionData::Fact(Some(f)) => fact(f),
            _ => {}
        },
        _ => {}
    }
    serde_json::to_value(e).unwrap()
}
