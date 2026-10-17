use std::io::Cursor;
use touchbar_protocol::{Mode, control::*};
#[test]
fn daemon_control_is_typed_versioned_and_bounded_to_64k() {
    for command in [
        Command::Status,
        Command::SetMode { mode: Mode::Native },
        Command::SetMode {
            mode: Mode::Display,
        },
        Command::Reload,
    ] {
        let request = Request::new(command);
        let bytes = encode(&request).unwrap();
        let got: Request = read_frame(&mut Cursor::new(bytes)).unwrap();
        got.validate().unwrap();
        assert_eq!(request, got);
    }
    for json in [
        r#"{"version":1,"command":{"kind":"sleep-pre"}}"#,
        r#"{"version":1,"command":{"kind":"set-mode","mode":"other"}}"#,
        r#"{"version":1,"command":{"kind":"status","extra":1}}"#,
        r#"{"version":1,"command":{"kind":"status"},"extra":1}"#,
    ] {
        assert!(
            serde_json::from_str::<Request>(json).is_err(),
            "accepted: {json}"
        );
    }
    let wrong = Request {
        version: 2,
        command: Command::Status,
    };
    assert!(wrong.validate().is_err());
    let error = Response::error("denied");
    let got: Response = read_frame(&mut Cursor::new(encode(&error).unwrap())).unwrap();
    got.validate().unwrap();
    assert_eq!(got, error);
    assert!(encode(&Response::error("x".repeat(MAX_FRAME))).is_err());
    let prefix = (u32::try_from(MAX_FRAME).unwrap() + 1).to_le_bytes();
    assert_eq!(
        read_frame::<_, Request>(&mut Cursor::new(prefix))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
}
