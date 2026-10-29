use quasar_tilemap::*;
use serde_json::{Value, json};
use std::path::PathBuf;
struct Options {
    command: String,
    map: PathBuf,
    output: Option<PathBuf>,
    assets: bool,
    tiled: bool,
    size: [u32; 2],
    camera: [f64; 2],
    time: u64,
    layer: Option<u32>,
    cell: [i32; 2],
    gid: Option<u32>,
    iterations: usize,
}
fn argument(args: &mut impl Iterator<Item = String>) -> Result<String> {
    args.next().ok_or_else(|| Error {
        code: "usage",
        message: "missing option argument".into(),
    })
}
fn parse_args() -> Result<Options> {
    let mut a = std::env::args().skip(1);
    let command = argument(&mut a)?;
    let map = argument(&mut a)?.into();
    let mut o = Options {
        command,
        map,
        output: None,
        assets: false,
        tiled: false,
        size: [320, 240],
        camera: [0., 0.],
        time: 0,
        layer: None,
        cell: [0, 0],
        gid: None,
        iterations: 1000,
    };
    let mut seen = std::collections::BTreeSet::new();
    while let Some(flag) = a.next() {
        if !seen.insert(flag.clone()) {
            return Err(Error {
                code: "usage",
                message: format!("duplicate option {flag}"),
            });
        }
        let invalid = |e: &dyn std::fmt::Display| Error {
            code: "usage",
            message: e.to_string(),
        };
        match flag.as_str() {
            "--output" => o.output = Some(argument(&mut a)?.into()),
            "--assets" => o.assets = true,
            "--tiled" => o.tiled = true,
            "--json" => {}
            "--size" => {
                o.size = [
                    argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
                    argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
                ];
            }
            "--camera" => {
                o.camera = [
                    argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
                    argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
                ];
            }
            "--time" => o.time = argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
            "--layer" => o.layer = Some(argument(&mut a)?.parse().map_err(|e| invalid(&e))?),
            "--cell" => {
                o.cell = [
                    argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
                    argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
                ]
            }
            "--gid" => o.gid = Some(argument(&mut a)?.parse().map_err(|e| invalid(&e))?),
            "--iterations" => o.iterations = argument(&mut a)?.parse().map_err(|e| invalid(&e))?,
            _ => {
                return Err(Error {
                    code: "usage",
                    message: format!("unknown option {flag}"),
                });
            }
        }
    }
    Ok(o)
}
fn run(o: Options) -> Result<Value> {
    let started = std::time::Instant::now();
    let (mut map, root) = load(&o.map)?;
    let runtime = Runtime::new(map.clone())?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.;
    if o.assets {
        validate_assets(&map, &root)?;
    }
    match o.command.as_str() {
        "validate" => Ok(
            json!({"ok":true,"version":map.quasar_version,"stored_cells":runtime.stored_cells(),"assets_checked":o.assets}),
        ),
        "inspect" => Ok(
            json!({"ok":true,"version":map.quasar_version,"tile_size":[map.tile_width,map.tile_height],"size":[map.width,map.height],"infinite":map.infinite,"stored_cells":runtime.stored_cells(),"tilesets":map.tilesets,"layers":map.layers.iter().map(|l|json!({"id":l.id,"name":l.name,"visible":l.visible,"properties":l.properties,"content":match &l.content{LayerContent::Tiles{chunks}=>json!({"kind":"tiles","chunks":chunks.len(),"cells":chunks.iter().map(|c|c.cells.len()).sum::<usize>()}),LayerContent::Objects{objects}=>json!({"kind":"objects","objects":objects})}})).collect::<Vec<_>>(),"properties":map.properties,"metadata":map.metadata}),
        ),
        "build" | "convert" | "set" => {
            if o.command == "set" {
                map.set_tile(
                    o.layer.ok_or_else(|| usage("set needs --layer"))?,
                    o.cell[0],
                    o.cell[1],
                    o.gid.ok_or_else(|| usage("set needs --gid"))?,
                )?;
            }
            #[cfg(feature = "png")]
            validate_assets(&map, &root)?;
            let output = o.output.ok_or_else(|| usage("command needs --output"))?;
            write_bundle(&map, &root, &output, o.tiled)?;
            Ok(json!({"ok":true,"output":output,"format":if o.tiled{"tiled-json"}else{"quasar-1"}}))
        }
        "preview" | "benchmark" => render_command(&o, &runtime, &root, load_ms),
        _ => Err(usage(
            "commands: validate, inspect, build, convert, set, preview, benchmark",
        )),
    }
}
fn usage(message: &str) -> Error {
    Error {
        code: "usage",
        message: message.into(),
    }
}
#[cfg(feature = "png")]
fn validate_assets(map: &Map, root: &AssetRoot) -> Result<()> {
    for path in map.asset_paths() {
        root.resolve(&path)?;
    }
    let mut textures = quasar_render_2d::Textures::default();
    render::LoadedTilesets::load(map, root, &mut textures)?;
    Ok(())
}
#[cfg(not(feature = "png"))]
fn validate_assets(_: &Map, _: &AssetRoot) -> Result<()> {
    Err(usage("asset PNG validation needs feature png"))
}
#[cfg(feature = "png")]
fn render_command(o: &Options, runtime: &Runtime, root: &AssetRoot, load_ms: f64) -> Result<Value> {
    use quasar_render_2d::{Canvas, Filter, Textures};
    use render::{DrawStats, LoadedTilesets, View};
    let view = View {
        x: o.camera[0],
        y: o.camera[1],
        width: o.size[0],
        height: o.size[1],
        zoom: 1.,
    };
    if o.command == "preview" {
        let output = o
            .output
            .as_ref()
            .ok_or_else(|| usage("preview needs --output"))?;
        let (pixels, stats) = render::preview(runtime, root, view, o.time)?;
        render::write_png(output, view.width, view.height, &pixels)?;
        return Ok(json!({"ok":true,"output":output,"stats":stats}));
    }
    if o.iterations == 0 || o.iterations > 100_000 {
        return Err(usage("iterations must be 1..100000"));
    }
    let mut textures = Textures::default();
    let atlases = LoadedTilesets::load(runtime.map(), root, &mut textures)?;
    let mut elapsed = Vec::with_capacity(o.iterations);
    let mut max_cells = 0;
    let mut max_quads = 0;
    for i in 0..o.iterations {
        let started = std::time::Instant::now();
        let mut canvas = Canvas::new(view.width, view.height).map_err(|e| usage(&e.to_string()))?;
        let mut stats = DrawStats::default();
        let world_width = runtime.map().width as f64 * runtime.map().tile_width as f64;
        let camera = View {
            x: if world_width > view.width as f64 {
                (i as f64 * 7.) % (world_width - view.width as f64)
            } else {
                view.x
            },
            ..view
        };
        for layer in &runtime.map().layers {
            stats += atlases.draw_layer(
                runtime,
                layer.id,
                &mut canvas,
                camera,
                o.time.wrapping_add(i as u64),
                Filter::Nearest,
            )?;
        }
        max_cells = max_cells.max(stats.visited_cells);
        max_quads = max_quads.max(stats.emitted_tiles);
        elapsed.push(started.elapsed().as_nanos() as u64);
    }
    elapsed.sort_unstable();
    Ok(
        json!({"ok":true,"iterations":o.iterations,"stored_cells":runtime.stored_cells(),"load_and_index_ms":load_ms,"max_visited_cells":max_cells,"max_emitted_tiles":max_quads,"draw_list_us":{"p50":elapsed[elapsed.len()/2]as f64/1000.,"p95":elapsed[(elapsed.len()-1)*95/100]as f64/1000.},"scope":"CPU indexed culling and draw-list construction; no GPU, decoding excluded"}),
    )
}
#[cfg(not(feature = "png"))]
fn render_command(_: &Options, _: &Runtime, _: &AssetRoot, _: f64) -> Result<Value> {
    Err(usage("preview/benchmark needs feature png"))
}
fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--help") {
        println!(
            "quasar-tilemap <validate|inspect|build|convert|set|preview|benchmark> MAP [--assets] [--output FILE] [--tiled] [--size W H] [--camera X Y] [--time MS] [--layer ID --cell X Y --gid GID] [--iterations N]\nAll results and errors are JSON; exit codes: 0 success, 1 invalid data, 2 usage."
        );
        return std::process::ExitCode::SUCCESS;
    }
    match parse_args().and_then(run) {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value).unwrap());
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", json!({"ok":false,"error":e}));
            std::process::ExitCode::from(if e.code == "usage" { 2 } else { 1 })
        }
    }
}
