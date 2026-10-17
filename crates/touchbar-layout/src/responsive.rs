#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Representation {
    Hidden,
    Minimal,
    Compact,
    Full,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponsiveVariant {
    pub representation: Representation,
    pub minimum_width: u32,
}
/// Select the richest fitting non-hidden representation, independent of
/// declaration order, matching the original UI's responsive contract.
pub fn resolve_responsive(variants: &[ResponsiveVariant], width: u32) -> Option<usize> {
    variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.representation != Representation::Hidden && v.minimum_width <= width)
        .max_by_key(|(_, v)| v.representation)
        .map(|(i, _)| i)
}
