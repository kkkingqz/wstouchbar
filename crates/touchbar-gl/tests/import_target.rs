use std::{
    fs::File,
    os::{fd::OwnedFd, unix::fs::FileExt},
    time::Duration,
};
use touchbar_gl::{Backend, DamageRect, Orientation, Renderer, TargetPath};
use touchbar_model::SurfaceId;
use touchbar_protocol::{SwapchainHeader, XRGB8888};
use touchbar_ui::{Color, DisplayList, MotionPolicy, Rect, Scene};
fn header(rotated: bool) -> SwapchainHeader {
    let (w, h) = if rotated { (4, 16) } else { (16, 4) };
    let pitch = w * 4 + 8;
    SwapchainHeader {
        logical_width: 16,
        logical_height: 4,
        physical_width: w,
        physical_height: h,
        format: XRGB8888,
        pitch,
        buffer_size: u64::from(pitch * h),
        buffer_count: 3,
        frame_damage: true,
    }
}
fn buffers(h: SwapchainHeader) -> (Vec<OwnedFd>, Vec<File>) {
    let mut fds = Vec::new();
    let mut files = Vec::new();
    for _ in 0..h.buffer_count {
        let fd = rustix::fs::memfd_create(
            "touchbar-gl-target",
            rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
        )
        .unwrap();
        let file = File::from(fd);
        file.set_len(h.buffer_size).unwrap();
        file.write_all_at(&vec![0xa5; usize::try_from(h.buffer_size).unwrap()], 0)
            .unwrap();
        rustix::fs::fcntl_add_seals(
            &file,
            rustix::fs::SealFlags::GROW
                | rustix::fs::SealFlags::SHRINK
                | rustix::fs::SealFlags::SEAL,
        )
        .unwrap();
        fds.push(file.try_clone().unwrap().into());
        files.push(file);
    }
    (fds, files)
}
fn packet(colors: [Color; 3]) -> DisplayList {
    let mut scene = Scene::new(Color::BLACK);
    for (i, c) in colors.into_iter().enumerate() {
        scene.rounded_rect(Rect::new((i * 5) as f32, 0.0, 2.0, 4.0), 0.0, c);
    }
    DisplayList::from_scene(&scene, Duration::ZERO, MotionPolicy::Disabled).unwrap()
}
fn expected(h: SwapchainHeader, rgba: &[u8]) -> Vec<u8> {
    let mut result = vec![0xa5; usize::try_from(h.buffer_size).unwrap()];
    for y in 0..h.logical_height {
        for x in 0..h.logical_width {
            let (px, py) =
                if (h.physical_width, h.physical_height) == (h.logical_width, h.logical_height) {
                    (x, y)
                } else {
                    (y, x)
                };
            let src = usize::try_from((y * h.logical_width + x) * 4).unwrap();
            let dst = usize::try_from(py * h.pitch + px * 4).unwrap();
            result[dst..dst + 4].copy_from_slice(&[rgba[src + 2], rgba[src + 1], rgba[src], 255]);
        }
    }
    result
}
fn bytes(file: &File, h: SwapchainHeader) -> Vec<u8> {
    let mut b = vec![0; usize::try_from(h.buffer_size).unwrap()];
    file.read_exact_at(&mut b, 0).unwrap();
    b
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_memfd_target_matches_landscape_reference_and_transposed_protocol_coordinates() {
    for rotated in [false, true] {
        let h = header(rotated);
        let (fds, files) = buffers(h);
        let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
        let scene = packet([
            Color::rgb(1.0, 0.0, 0.0),
            Color::rgb(0.0, 1.0, 0.0),
            Color::rgb(0.0, 0.0, 1.0),
        ]);
        let reference = renderer
            .render(SurfaceId(1), (16, 4), Orientation::Landscape, &scene, true)
            .unwrap()
            .unwrap();
        let mut target = renderer.import_target(fds, h).unwrap();
        assert_eq!(target.path(), TargetPath::Copy);
        assert!(target.warning().is_some());
        let frame = renderer
            .render_target(SurfaceId(2), &mut target, 0, &scene, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            frame.copied,
            vec![DamageRect {
                x: 0,
                y: 0,
                width: h.physical_width,
                height: h.physical_height
            }]
        );
        assert_eq!(bytes(&files[0], h), expected(h, &reference.pixels));
        assert_eq!(
            bytes(&files[1], h),
            vec![0xa5; usize::try_from(h.buffer_size).unwrap()]
        );
    }
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_copy_target_reused_buffers_contain_all_intervening_changes() {
    let h = header(true);
    let (fds, files) = buffers(h);
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    let mut target = renderer.import_target(fds, h).unwrap();
    let mut colors = [Color::BLACK; 3];
    for (n, index) in [0u32, 1, 2, 0, 2, 1, 0].into_iter().enumerate() {
        colors[n % 3] = Color::rgb((n as f32 + 1.0) / 8.0, 0.25, 0.75);
        let scene = packet(colors);
        let reference = renderer
            .render(SurfaceId(1), (16, 4), Orientation::Landscape, &scene, true)
            .unwrap()
            .unwrap();
        renderer
            .render_target(SurfaceId(2), &mut target, index, &scene, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            bytes(&files[index as usize], h),
            expected(h, &reference.pixels)
        );
    }
    // Reuse the latest target buffer: only rows corresponding to the changed logical columns.
    colors[0] = Color::WHITE;
    let scene = packet(colors);
    let before = bytes(&files[0], h);
    let reference = renderer
        .render(SurfaceId(1), (16, 4), Orientation::Landscape, &scene, true)
        .unwrap()
        .unwrap();
    let reference = expected(h, &reference.pixels);
    // Derive exact dirty rows from the rendered reference, including antialiased edges.
    let dirty = (0..h.physical_height)
        .filter(|y| {
            let start = usize::try_from(y * h.pitch).unwrap();
            let end = start + usize::try_from(h.physical_width * 4).unwrap();
            before[start..end] != reference[start..end]
        })
        .collect::<Vec<_>>();
    assert!(!dirty.is_empty());
    let first = *dirty.first().unwrap();
    let end = *dirty.last().unwrap() + 1;
    let update = renderer
        .render_target(SurfaceId(2), &mut target, 0, &scene, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        update.copied,
        vec![DamageRect {
            x: 0,
            y: first,
            width: h.physical_width,
            height: end - first
        }]
    );
    let after = bytes(&files[0], h);
    assert_eq!(after, reference);
    for y in 0..h.physical_height {
        if !dirty.contains(&y) {
            let start = usize::try_from(y * h.pitch).unwrap();
            let end = start + usize::try_from(h.pitch).unwrap();
            assert_eq!(before[start..end], after[start..end]);
        }
    }
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_copy_target_lost_history_copies_full_current_image() {
    let h = header(true);
    let (fds, files) = buffers(h);
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    let mut target = renderer.import_target(fds, h).unwrap();
    let mut scene = packet([Color::BLACK; 3]);
    renderer
        .render_target(SurfaceId(1), &mut target, 0, &scene, false)
        .unwrap()
        .unwrap();
    for n in 1..70 {
        scene = packet([
            Color::rgb(n as f32 / 80.0, 0.0, 0.0),
            Color::BLACK,
            Color::BLACK,
        ]);
        renderer
            .render_target(SurfaceId(1), &mut target, 1 + n % 2, &scene, false)
            .unwrap()
            .unwrap();
    }
    scene = packet([Color::WHITE, Color::BLACK, Color::BLACK]);
    let reference = renderer
        .render(SurfaceId(2), (16, 4), Orientation::Landscape, &scene, true)
        .unwrap()
        .unwrap();
    let update = renderer
        .render_target(SurfaceId(1), &mut target, 0, &scene, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        update.copied,
        vec![DamageRect {
            x: 0,
            y: 0,
            width: 4,
            height: 16
        }]
    );
    assert_eq!(bytes(&files[0], h), expected(h, &reference.pixels));
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_import_target_validates_fds_and_renderer_ownership() {
    let h = header(false);
    let mut first = Renderer::new(Backend::Surfaceless).unwrap();
    assert!(first.import_target(Vec::new(), h).is_err());
    let (fds, _) = buffers(h);
    let mut target = first.import_target(fds, h).unwrap();
    let mut second = Renderer::new(Backend::Surfaceless).unwrap();
    assert!(
        second
            .render_target(
                SurfaceId(1),
                &mut target,
                0,
                &packet([Color::BLACK; 3]),
                false
            )
            .is_err()
    );
    assert!(
        first
            .render_target(
                SurfaceId(1),
                &mut target,
                3,
                &packet([Color::BLACK; 3]),
                false
            )
            .is_err()
    );
    first
        .render_target(
            SurfaceId(1),
            &mut target,
            0,
            &packet([Color::WHITE; 3]),
            false,
        )
        .unwrap();
    drop(first);
    drop(target);
    assert!(
        second
            .render(
                SurfaceId(1),
                (16, 4),
                Orientation::Landscape,
                &packet([Color::WHITE; 3]),
                true
            )
            .is_ok()
    );
}

#[path = "common/dma.rs"]
mod dma;
#[test]
#[ignore = "requires render node and EGL dma-buf import; CPU-only environments report a skip"]
fn env_gpu_dma_buf_import_with_a_render_node() {
    const CHILD: &str = "WSTOUCHBAR_DMA_IMPORT_TEST_NODE";
    let node = if let Some(path) = std::env::var_os(CHILD) {
        std::path::PathBuf::from(path)
    } else {
        let mut paths = std::fs::read_dir("/dev/dri")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("renderD"))
            })
            .collect::<Vec<_>>();
        paths.sort();
        let Some(node) = paths.into_iter().next() else {
            eprintln!(
                "SKIP dma-buf GPU import: no render node; memfd copy path is covered separately"
            );
            return;
        };
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "env_gpu_dma_buf_import_with_a_render_node",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD, &node)
            .env_remove("LIBGL_ALWAYS_SOFTWARE")
            .status()
            .unwrap();
        assert!(status.success(), "real-node import child failed");
        return;
    };
    let mut renderer = Renderer::new(Backend::RenderNode(node)).unwrap();
    for rotated in [false, true] {
        let physical = if rotated { (4, 16) } else { (16, 4) };
        let fixture = dma::Gbm::new(
            std::path::Path::new(&std::env::var_os(CHILD).unwrap()),
            physical.0,
            physical.1,
        )
        .unwrap();
        let (h, fds) = fixture.export((16, 4), physical).unwrap();
        let readback = rustix::io::dup(&fds[0]).unwrap();
        let mut target = renderer.import_target(fds, h).unwrap();
        assert_eq!(target.path(), TargetPath::Direct, "{:?}", target.warning());
        let scene = packet([
            Color::rgb(1.0, 0.0, 0.0),
            Color::rgb(0.0, 1.0, 0.0),
            Color::rgb(0.0, 0.0, 1.0),
        ]);
        let reference = renderer
            .render(SurfaceId(1), (16, 4), Orientation::Landscape, &scene, true)
            .unwrap()
            .unwrap();
        renderer
            .render_target(SurfaceId(2), &mut target, 0, &scene, false)
            .unwrap()
            .unwrap();
        let actual = dma::read(&readback, h.buffer_size).unwrap();
        let expected = expected(h, &reference.pixels);
        for y in 0..h.physical_height {
            for x in 0..h.physical_width {
                let i = usize::try_from(y * h.pitch + x * 4).unwrap();
                assert_eq!(
                    actual[i..i + 3],
                    expected[i..i + 3],
                    "physical pixel {x},{y}"
                );
            }
        }
        eprintln!(
            "dma-buf direct RGB verified; {}",
            renderer.backend_description()
        );
    }
}

#[path = "common/hardware.rs"]
mod hardware;
#[test]
#[ignore = "requires root, isolated appletbdrm, and explicit H11+H14/H18 hardware approval"]
fn env_hardware_appletbdrm_buffers_import_into_gpu() -> std::io::Result<()> {
    if let Some((h, buffers)) = hardware::child_buffers()? {
        let readback = rustix::io::dup(&buffers[0])?;
        let mut renderer = Renderer::new(Backend::Auto).map_err(std::io::Error::other)?;
        let scene = packet([
            Color::rgb(1.0, 0.0, 0.0),
            Color::rgb(0.0, 1.0, 0.0),
            Color::rgb(0.0, 0.0, 1.0),
        ]);
        let reference = renderer
            .render(
                SurfaceId(1),
                (h.logical_width, h.logical_height),
                Orientation::Landscape,
                &scene,
                true,
            )
            .map_err(std::io::Error::other)?
            .ok_or_else(|| std::io::Error::other("missing first reference frame"))?;
        let mut target = renderer
            .import_target(buffers, h)
            .map_err(std::io::Error::other)?;
        renderer
            .render_target(SurfaceId(2), &mut target, 0, &scene, false)
            .map_err(std::io::Error::other)?
            .ok_or_else(|| std::io::Error::other("missing first imported frame"))?;
        let actual = dma::read(&readback, h.buffer_size).map_err(std::io::Error::other)?;
        let expected = expected(h, &reference.pixels);
        for y in 0..h.physical_height {
            for x in 0..h.physical_width {
                let i = usize::try_from(y * h.pitch + x * 4).map_err(std::io::Error::other)?;
                assert_eq!(
                    actual[i..i + 3],
                    expected[i..i + 3],
                    "physical pixel {x},{y}"
                );
            }
        }
        eprintln!(
            "appletbdrm import path={:?}; warning={:?}; {}",
            target.path(),
            target.warning(),
            renderer.backend_description()
        );
        return Ok(());
    }
    hardware::with_buffers("env_hardware_appletbdrm_buffers_import_into_gpu")
}
