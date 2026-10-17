use touchbar_gl::*;
#[test]
fn png_export_preserves_top_down_order_and_converts_premultiplied_alpha() {
    let path = std::env::temp_dir().join(format!("wstouchbar-gl-{}.png", std::process::id()));
    let frame = RenderedFrame {
        width: 1,
        height: 2,
        pixels: vec![128, 0, 0, 128, 0, 255, 0, 255],
        damage: vec![],
    };
    frame.save_png(&path).unwrap();
    let mut decoder =
        png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
            .read_info()
            .unwrap();
    let mut output = vec![0; decoder.output_buffer_size().unwrap()];
    let info = decoder.next_frame(&mut output).unwrap();
    assert_eq!((info.width, info.height), (1, 2));
    assert_eq!(output, vec![255, 0, 0, 128, 0, 255, 0, 255]);
    std::fs::remove_file(path).unwrap();
}
