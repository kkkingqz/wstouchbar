use std::path::{Path, PathBuf};
pub(crate) fn preferred_node(
    explicit: Option<PathBuf>,
    mut nodes: Vec<PathBuf>,
    boots: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    nodes.sort();
    let boot = nodes.iter().position(|path| boots(path));
    boot.map(|index| nodes.swap_remove(index))
        .or_else(|| nodes.into_iter().next())
}
pub(crate) fn discover() -> Option<PathBuf> {
    let nodes = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("renderD"))
        })
        .collect();
    preferred_node(
        std::env::var_os("TOUCHBAR_RENDER_NODE").map(PathBuf::from),
        nodes,
        |node| {
            node.file_name().is_some_and(|name| {
                std::fs::read_to_string(
                    Path::new("/sys/class/drm")
                        .join(name)
                        .join("device/boot_vga"),
                )
                .is_ok_and(|s| s.trim() == "1")
            })
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn nodes() -> Vec<PathBuf> {
        vec!["/dev/dri/renderD129".into(), "/dev/dri/renderD128".into()]
    }
    #[test]
    fn explicit_override_wins() {
        assert_eq!(
            preferred_node(Some("/chosen".into()), nodes(), |_| true),
            Some("/chosen".into())
        );
    }
    #[test]
    fn boot_gpu_wins_over_sorted_first() {
        assert_eq!(
            preferred_node(None, nodes(), |p| p.ends_with("renderD129")),
            Some("/dev/dri/renderD129".into())
        );
    }
    #[test]
    fn fallback_is_sorted_and_empty_nodes_allow_surfaceless() {
        assert_eq!(
            preferred_node(None, nodes(), |_| false),
            Some("/dev/dri/renderD128".into())
        );
        assert_eq!(preferred_node(None, vec![], |_| true), None);
    }
}
