use crate::device::Device;
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};
pub struct ModeWriter {
    file: File,
}
impl ModeWriter {
    pub fn open(device: &Device, path: &Path) -> io::Result<Self> {
        let dir = device.open_directory()?;
        let relative = path.strip_prefix(device.path()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "HID mode outside selected Touch Bar",
            )
        })?;
        if relative.file_name() != Some(std::ffi::OsStr::new("mode")) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let fd = openat2(
            dir,
            relative,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )?;
        Ok(Self { file: fd.into() })
    }
    pub fn read_mode(&mut self) -> io::Result<u8> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = [0; 16];
        let n = self.file.read(&mut bytes)?;
        let s = std::str::from_utf8(&bytes[..n]).map_err(|_| io::ErrorKind::InvalidData)?;
        match s.trim() {
            "0" => Ok(0),
            "1" => Ok(1),
            "2" => Ok(2),
            "3" => Ok(3),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
    pub fn write_mode(&mut self, mode: u8) -> io::Result<()> {
        if mode > 3 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&[b'0' + mode, b'\n'])
    }
}
