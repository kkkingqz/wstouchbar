use std::{fs, io, time::Duration};
use touchbard::device::{FsMonitor, Uevent};
#[test]
fn kernel_event_parser_requires_consistent_identity_and_canonical_devpath() {
    let event=Uevent::decode(b"add@/devices/pci/usb1/1-1\0ACTION=add\0DEVPATH=/devices/pci/usb1/1-1\0SUBSYSTEM=usb\0PRODUCT=5ac/8302/200\0").unwrap();
    assert_eq!(Uevent::from_kernel(123, b"invalid").unwrap(), None);
    assert!(event.relevant());
    assert_eq!(event.action, "add");
    assert_eq!(event.devpath, std::path::Path::new("/devices/pci/usb1/1-1"));
    for bytes in [
        b"add@/devices/x\0ACTION=remove\0DEVPATH=/devices/x\0SUBSYSTEM=usb\0".as_slice(),
        b"add@/devices/x\0ACTION=add\0DEVPATH=/devices/y\0SUBSYSTEM=usb\0",
        b"add@/devices/../etc\0ACTION=add\0DEVPATH=/devices/../etc\0SUBSYSTEM=usb\0",
        b"add@/devices/x\0ACTION=add\0DEVPATH=/devices/x\0",
        b"add@/devices/x\0ACTION=add\0ACTION=add\0DEVPATH=/devices/x\0SUBSYSTEM=usb\0",
        b"incomplete",
    ] {
        assert!(Uevent::decode(bytes).is_err());
    }
    assert!(Uevent::decode(&vec![b'x'; 65537]).is_err());
    assert!(!Uevent::decode(b"change@/devices/pci/x\0ACTION=change\0DEVPATH=/devices/pci/x\0SUBSYSTEM=power_supply\0").unwrap().relevant());
}
#[test]
fn missing_watch_targets_filter_ancestor_changes_and_can_rearm_on_creation() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target/child");
    let watch = FsMonitor::new(std::slice::from_ref(&target)).unwrap();
    assert_eq!(
        watch.try_changed().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    fs::write(dir.path().join("unrelated"), "write").unwrap();
    assert!(!watch.try_changed().unwrap());
    fs::create_dir(dir.path().join("target")).unwrap();
    assert!(watch.try_changed().unwrap());
    let watch = FsMonitor::new(std::slice::from_ref(&target)).unwrap();
    fs::create_dir(&target).unwrap();
    assert!(watch.try_changed().unwrap());
    let watch = FsMonitor::new(std::slice::from_ref(&target)).unwrap();
    futures_lite::future::block_on(async {
        fs::write(target.join("mode"), "1").unwrap();
        futures_lite::future::or(
            async {
                watch.next().await.unwrap();
            },
            async {
                async_io::Timer::after(Duration::from_secs(1)).await;
                panic!("no filesystem notification");
            },
        )
        .await;
    });
}
