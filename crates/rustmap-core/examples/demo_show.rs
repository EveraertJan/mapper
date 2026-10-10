//! Writes a small demo show.
//!
//! cargo run -p rustmap-core --example demo_show -- demo.rustmap [--production] [--m5] [--led] [--dmx] [image.png]
//!
//! `--m5` adds a second surface with a bent mesh, a mask and color
//! correction, and a right-hand edge blend on the output. `--led` adds an
//! Art-Net strip and an sACN matrix sending to 127.0.0.1. `--dmx` adds four
//! RGBW PARs and a moving head on Art-Net universe 1, also to 127.0.0.1.

use std::path::PathBuf;

use rustmap_core::{FixtureShape, Mask, Mesh, Project, Protocol, TestPattern, file};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().map_or("demo.rustmap", String::as_str));
    let production = args.iter().any(|a| a == "--production");
    let m5 = args.iter().any(|a| a == "--m5");
    let led = args.iter().any(|a| a == "--led");
    let dmx = args.iter().any(|a| a == "--dmx");
    let image = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from);

    let mut p = Project::default();
    p.name = "Demo".into();
    p.production = production;
    let o = p.add_output();
    p.output_mut(o).unwrap().test_pattern = TestPattern::Grid;
    let s = p.add_surface(o).unwrap();
    p.surface_mut(s).unwrap().corners = [[0.15, 0.1], [0.85, 0.2], [0.8, 0.9], [0.2, 0.8]];
    if let Some(img) = image {
        let path = std::fs::canonicalize(&img).unwrap_or(img);
        let m = p.add_image(path);
        p.surface_mut(s).unwrap().media = Some(m);
        if m5 {
            let s2 = p.add_surface(o).unwrap();
            let surface = p.surface_mut(s2).unwrap();
            surface.media = Some(m);
            // A 3x3 mesh over the right half with its middle point pushed up.
            let mut points = Vec::new();
            for r in 0..3 {
                for c in 0..3 {
                    points.push([0.55 + 0.2 * c as f32, 0.15 + 0.35 * r as f32]);
                }
            }
            points[4] = [0.7, 0.35];
            surface.mesh = Some(Mesh {
                cols: 3,
                rows: 3,
                points,
                smooth: true,
            });
            surface.sync_corners_from_mesh();
            surface.mask = Some(Mask {
                points: vec![[0.1, 0.1], [0.9, 0.2], [0.8, 0.9], [0.2, 0.75]],
                feather: 0.05,
                invert: false,
            });
            // Color shader: brightness, contrast, saturation, hue, R, G, B.
            surface.filters = vec![rustmap_core::FilterUse {
                shader: "color".into(),
                enabled: true,
                params: vec![0.0, 1.0, 1.5, 120.0, 1.0, 1.0, 1.0],
            }];
            surface.name = "Bent".into();
            p.output_mut(o).unwrap().edge_blend.right = 0.2;
        }
    }
    if led {
        let strip = p
            .add_fixture(
                o,
                FixtureShape::Strip {
                    start: [0.05, 0.95],
                    end: [0.95, 0.95],
                    pixels: 30,
                },
            )
            .unwrap();
        let matrix = p
            .add_fixture(o, rustmap_core::Fixture::default_matrix())
            .unwrap();
        for id in [strip, matrix] {
            p.fixture_mut(id).unwrap().patch.destination = "127.0.0.1".into();
        }
        let m = p.fixture_mut(matrix).unwrap();
        m.patch.protocol = Protocol::Sacn;
        m.patch.universe = 1;
        m.patch.channel = 1;
        p.led.send = true;
    }
    if dmx {
        let templates = rustmap_core::DmxType::templates();
        let par = p.add_dmx_type(templates[2].clone());
        let head = p.add_dmx_type(templates[3].clone());
        for i in 0..4 {
            let id = p.add_dmx_fixture(par).unwrap();
            let f = p.dmx_fixture_mut(id).unwrap();
            if i == 0 {
                f.patch.universe = 1;
                f.patch.destination = "127.0.0.1".into();
            }
            f.values[0] = 255;
            f.values[1 + i % 3] = 255;
        }
        p.add_dmx_fixture(head).unwrap();
        p.led.send = true;
    }
    file::save(&p, &out).expect("write show");
    println!("wrote {}", out.display());
}
