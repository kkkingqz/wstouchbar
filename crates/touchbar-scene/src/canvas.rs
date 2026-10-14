use anyhow::{Context, Result, bail, ensure};
use u::Point;
#[derive(Default)]
pub(crate) struct Budget {
    commands: usize,
    points: usize,
    segments: usize,
    triangles: usize,
}
use touchbar_model as m;
use touchbar_ui as u;
fn point(p: m::Point) -> u::Point {
    u::Point::new(p.x, p.y)
}
pub(crate) fn rect(r: m::Rect) -> u::Rect {
    u::Rect::new(r.x, r.y, r.width, r.height)
}
fn role(r: m::ColorRole) -> u::ColorRole {
    match r {
        m::ColorRole::Background => u::ColorRole::Background,
        m::ColorRole::Control => u::ColorRole::Control,
        m::ColorRole::ControlPressed => u::ColorRole::ControlPressed,
        m::ColorRole::Track => u::ColorRole::Track,
        m::ColorRole::Foreground => u::ColorRole::Foreground,
        m::ColorRole::Muted => u::ColorRole::Muted,
        m::ColorRole::Accent => u::ColorRole::Accent,
        m::ColorRole::Destructive => u::ColorRole::Destructive,
        m::ColorRole::OnAccent => u::ColorRole::OnAccent,
        m::ColorRole::OnDestructive => u::ColorRole::OnDestructive,
    }
}
fn paint(p: m::CanvasPaint) -> u::CanvasPaint {
    u::CanvasPaint {
        opacity: p.opacity,
        color: match p.color {
            m::CanvasColor::Role(r) => u::CanvasColor::Role(role(r)),
            m::CanvasColor::Rgba {
                red,
                green,
                blue,
                alpha,
            } => u::CanvasColor::Rgba(u::Color::rgba(red, green, blue, alpha)),
        },
    }
}
pub(crate) fn convert(
    commands: &[m::CanvasCommand],
    budget: &mut Budget,
    scale: f32,
) -> Result<Vec<u::CanvasCommand>> {
    budget.commands += commands.len();
    ensure!(budget.commands <= 512, "canvas command budget exceeded");
    commands
        .iter()
        .map(|c| {
            Ok(match c {
                m::CanvasCommand::FillRect {
                    rect: r,
                    radius,
                    paint: p,
                } => u::CanvasCommand::FillRect {
                    rect: rect(*r),
                    radius: *radius,
                    paint: paint(*p),
                },
                m::CanvasCommand::Line {
                    start,
                    end,
                    width,
                    paint: p,
                } => u::CanvasCommand::Line {
                    start: point(*start),
                    end: point(*end),
                    width: *width,
                    paint: paint(*p),
                },
                m::CanvasCommand::Polyline {
                    points,
                    width,
                    paint: p,
                } => {
                    budget.points += points.len();
                    ensure!(budget.points <= 2048, "canvas point budget exceeded");
                    u::CanvasCommand::Polyline {
                        points: points.iter().copied().map(point).collect(),
                        width: *width,
                        paint: paint(*p),
                    }
                }
                m::CanvasCommand::FillCircle {
                    center,
                    radius,
                    paint: p,
                } => u::CanvasCommand::FillCircle {
                    center: point(*center),
                    radius: *radius,
                    paint: paint(*p),
                },
                m::CanvasCommand::Text {
                    rect: r,
                    text,
                    size,
                    paint: p,
                    align,
                } => {
                    ensure!(
                        size.is_finite() && *size > 0.0 && size * scale <= 512.0,
                        "canvas font size exceeds budget"
                    );
                    u::CanvasCommand::Text {
                        rect: rect(*r),
                        text: text.clone(),
                        size: *size,
                        paint: paint(*p),
                        align: match align {
                            m::TextAlign::Leading => u::TextAlign::Leading,
                            m::TextAlign::Center => u::TextAlign::Center,
                            m::TextAlign::Trailing => u::TextAlign::Trailing,
                        },
                    }
                }
                m::CanvasCommand::FillLinearGradientRect {
                    rect: r,
                    radius,
                    gradient,
                } => u::CanvasCommand::FillLinearGradientRect {
                    rect: rect(*r),
                    radius: *radius,
                    gradient: u::CanvasLinearGradient {
                        start: point(gradient.start),
                        end: point(gradient.end),
                        start_color: paint(gradient.start_color),
                        end_color: paint(gradient.end_color),
                    },
                },
                m::CanvasCommand::StrokePath {
                    segments,
                    width,
                    paint: p,
                } => {
                    let paths = flatten(segments, false, budget)?;
                    u::CanvasCommand::StrokePath {
                        subpaths: paths.into_iter().map(std::sync::Arc::from).collect(),
                        width: *width,
                        paint: paint(*p),
                    }
                }
                m::CanvasCommand::FillPath { segments, paint: p } => {
                    let mut paths = flatten(segments, true, budget)?;
                    let mut points = paths.remove(0);
                    if points.last() == points.first() {
                        points.pop();
                    }
                    simplify_collinear_polygon(&mut points);
                    ensure!(
                        (3..=258).contains(&points.len()),
                        "canvas fill vertex budget exceeded"
                    );
                    validate_simple_polygon(&points)?;
                    let triangles = triangulate_simple_polygon(&points)?;
                    budget.triangles += triangles.len() / 3;
                    ensure!(
                        budget.triangles <= 512,
                        "canvas fill triangle budget exceeded"
                    );
                    u::CanvasCommand::FillPath {
                        triangles: triangles.into(),
                        paint: paint(*p),
                    }
                }
            })
        })
        .collect()
}

fn flatten(
    segments: &[m::CanvasPathSegment],
    fill: bool,
    budget: &mut Budget,
) -> Result<Vec<Vec<Point>>> {
    ensure!(
        (if fill { 4 } else { 2 }..=if fill { 128 } else { 256 }).contains(&segments.len()),
        "canvas path segment count outside bounds"
    );
    budget.segments += segments.len();
    ensure!(
        budget.segments <= 256,
        "canvas path segment budget exceeded"
    );
    let mut paths = Vec::new();
    let mut points = Vec::new();
    let mut current = None;
    let mut start = None;
    let mut closed = false;
    for segment in segments {
        ensure!(
            !fill || !closed,
            "canvas fill needs exactly one closed subpath"
        );
        match segment {
            m::CanvasPathSegment::MoveTo(p) => {
                ensure!(!fill || points.is_empty(), "canvas fill needs one subpath");
                if points.len() >= 2 {
                    paths.push(std::mem::take(&mut points));
                } else {
                    points.clear();
                }
                let p = point(*p);
                points.push(p);
                current = Some(p);
                start = Some(p);
            }
            m::CanvasPathSegment::LineTo(p) => {
                ensure!(current.is_some(), "canvas path needs move-to");
                let p = point(*p);
                points.push(p);
                current = Some(p);
            }
            m::CanvasPathSegment::QuadraticTo { control, endpoint } => {
                let from = current.context("canvas path needs move-to")?;
                let control = point(*control);
                let endpoint = point(*endpoint);
                for step in 1..=8 {
                    let t = step as f32 / 8.0;
                    let inv = 1.0 - t;
                    points.push(Point::new(
                        inv * inv * from.x + 2.0 * inv * t * control.x + t * t * endpoint.x,
                        inv * inv * from.y + 2.0 * inv * t * control.y + t * t * endpoint.y,
                    ));
                }
                current = Some(endpoint);
            }
            m::CanvasPathSegment::CubicTo {
                control_one,
                control_two,
                endpoint,
            } => {
                let from = current.context("canvas path needs move-to")?;
                let a = point(*control_one);
                let b = point(*control_two);
                let end = point(*endpoint);
                for step in 1..=12 {
                    let t = step as f32 / 12.0;
                    let inv = 1.0 - t;
                    points.push(Point::new(
                        inv.powi(3) * from.x
                            + 3.0 * inv * inv * t * a.x
                            + 3.0 * inv * t * t * b.x
                            + t.powi(3) * end.x,
                        inv.powi(3) * from.y
                            + 3.0 * inv * inv * t * a.y
                            + 3.0 * inv * t * t * b.y
                            + t.powi(3) * end.y,
                    ));
                }
                current = Some(end);
            }
            m::CanvasPathSegment::Close => {
                let first = start.context("canvas path close needs move-to")?;
                ensure!(current.is_some(), "canvas path close needs open path");
                if points.last() != Some(&first) {
                    points.push(first);
                }
                if !fill {
                    paths.push(std::mem::take(&mut points));
                }
                closed = true;
                current = None;
                start = None;
            }
        }
    }
    ensure!(!fill || closed, "canvas fill must end with close");
    if points.len() >= 2 {
        paths.push(points);
    }
    ensure!(!paths.is_empty(), "canvas path has no drawable subpath");
    let count = paths.iter().map(Vec::len).sum::<usize>();
    budget.points += count;
    ensure!(budget.points <= 2048, "canvas point budget exceeded");
    ensure!(
        paths
            .iter()
            .flatten()
            .all(|p| p.x.is_finite() && p.y.is_finite()),
        "non-finite flattened canvas coordinates"
    );
    Ok(paths)
}

// CPU polygon validation and ear clipping ported from the original plugin-host.
fn polygon_cross(a: Point, b: Point, c: Point) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn polygon_area(points: &[Point]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left.x * right.y - right.x * left.y)
        .sum::<f32>()
        * 0.5
}

fn simplify_collinear_polygon(points: &mut Vec<Point>) {
    loop {
        if points.len() <= 3 {
            break;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let previous = points[(index + points.len() - 1) % points.len()];
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            if current == previous
                || current == next
                || polygon_cross(previous, current, next) == 0.0
            {
                points.remove(index);
                removed = true;
                break;
            }
        }
        if !removed {
            break;
        }
    }
}

fn validate_simple_polygon(points: &[Point]) -> Result<()> {
    for left in 0..points.len() {
        for right in left + 1..points.len() {
            if points[left] == points[right] {
                bail!("canvas fill path repeats a vertex");
            }
        }
    }
    for left in 0..points.len() {
        let left_next = (left + 1) % points.len();
        for right in left + 1..points.len() {
            let right_next = (right + 1) % points.len();
            if left == right || left_next == right || right_next == left {
                continue;
            }
            if segments_intersect(
                points[left],
                points[left_next],
                points[right],
                points[right_next],
            ) {
                bail!("canvas fill path self-intersects");
            }
        }
    }
    if polygon_area(points).abs() < 0.0001 {
        bail!("canvas fill path has zero area");
    }
    Ok(())
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab_c = polygon_cross(a, b, c);
    let ab_d = polygon_cross(a, b, d);
    let cd_a = polygon_cross(c, d, a);
    let cd_b = polygon_cross(c, d, b);
    (ab_c == 0.0 && point_on_segment(c, a, b))
        || (ab_d == 0.0 && point_on_segment(d, a, b))
        || (cd_a == 0.0 && point_on_segment(a, c, d))
        || (cd_b == 0.0 && point_on_segment(b, c, d))
        || ((ab_c > 0.0) != (ab_d > 0.0) && (cd_a > 0.0) != (cd_b > 0.0))
}

fn point_on_segment(point: Point, start: Point, end: Point) -> bool {
    point.x >= start.x.min(end.x)
        && point.x <= start.x.max(end.x)
        && point.y >= start.y.min(end.y)
        && point.y <= start.y.max(end.y)
}

fn triangulate_simple_polygon(points: &[Point]) -> Result<Vec<Point>> {
    let counter_clockwise = polygon_area(points) > 0.0;
    let mut indices = (0..points.len()).collect::<Vec<_>>();
    let mut triangles = Vec::with_capacity((points.len() - 2) * 3);
    while indices.len() > 3 {
        let mut ear = None;
        for position in 0..indices.len() {
            let previous = indices[(position + indices.len() - 1) % indices.len()];
            let current = indices[position];
            let next = indices[(position + 1) % indices.len()];
            let cross = polygon_cross(points[previous], points[current], points[next]);
            if (counter_clockwise && cross <= 0.0) || (!counter_clockwise && cross >= 0.0) {
                continue;
            }
            if indices.iter().copied().any(|candidate| {
                candidate != previous
                    && candidate != current
                    && candidate != next
                    && point_in_triangle(
                        points[candidate],
                        points[previous],
                        points[current],
                        points[next],
                        counter_clockwise,
                    )
            }) {
                continue;
            }
            ear = Some((position, previous, current, next));
            break;
        }
        let Some((position, previous, current, next)) = ear else {
            bail!("canvas fill path could not be triangulated");
        };
        triangles.extend_from_slice(&[points[previous], points[current], points[next]]);
        indices.remove(position);
    }
    triangles.extend_from_slice(&[points[indices[0]], points[indices[1]], points[indices[2]]]);
    Ok(triangles)
}

fn point_in_triangle(point: Point, a: Point, b: Point, c: Point, counter_clockwise: bool) -> bool {
    let signs = [
        polygon_cross(a, b, point),
        polygon_cross(b, c, point),
        polygon_cross(c, a, point),
    ];
    if counter_clockwise {
        signs.into_iter().all(|value| value >= 0.0)
    } else {
        signs.into_iter().all(|value| value <= 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversized_canvas_font_before_shaping() {
        let command = m::CanvasCommand::Text {
            rect: m::Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            text: "Huge".into(),
            size: 100_000.0,
            paint: m::CanvasPaint {
                color: m::CanvasColor::Role(m::ColorRole::Accent),
                opacity: 1.0,
            },
            align: m::TextAlign::Center,
        };
        assert!(convert(&[command], &mut Budget::default(), 1.0).is_err());
    }
}
