//! Game-specific quest rules and movement. Quasar owns maps, assets and rendering.
pub mod controls;
use quasar_render_2d::{Camera2D, Canvas, Color, Filter, Rect, SpriteAtlas, Textures};
use quasar_tilemap::render::{DrawStats, LoadedTilesets, View};
use quasar_tilemap::{AssetRoot, Bounds, LayerContent, Runtime};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
pub const WIDTH: u32 = 480;
pub const HEIGHT: u32 = 320;
pub const HZ: f64 = 60.;
pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub x: i8,
    pub y: i8,
    pub pause: bool,
    pub restart: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Playing,
    Paused,
    Won,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub position: [i32; 2],
    pub facing_left: bool,
    pub moving: bool,
    pub play_ticks: u64,
    pub collected: BTreeSet<u32>,
    pub phase: Phase,
}
pub struct World {
    pub runtime: Runtime,
    pub root: AssetRoot,
    pub ground: u32,
    pub collision: u32,
    pub markers: u32,
    pub spawn: [i32; 2],
    pub beacons: Vec<(u32, [i32; 2])>,
    pub exit: [i32; 2],
    pub hash: u64,
}
impl World {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let (map, root) = quasar_tilemap::load(path)?;
        if map.width > 4096
            || map.height > 4096
            || u64::from(map.width) * u64::from(map.height) > quasar_tilemap::MAX_CELLS as u64
        {
            return Err("quest navigation grid exceeds supported dimensions".into());
        }
        if map.infinite
            || map.tile_width != 16
            || map.tile_height != 16
            || map.tilesets.first().is_none_or(|s| s.tile_count < 32)
        {
            return Err(
                "Tilequest needs a finite 16px map and an atlas containing its 32 game sprites"
                    .into(),
            );
        }
        let role = |name: &str| {
            map.layers
                .iter()
                .find(|l| l.name == name)
                .map(|l| l.id)
                .ok_or_else(|| format!("Tilequest map is missing layer {name}"))
        };
        let ground = role("ground")?;
        let collision = role("collision")?;
        let markers = role("markers")?;
        let runtime = Runtime::new(map)?;
        let objects = runtime.objects(markers)?;
        let point = |o: &quasar_tilemap::Object| -> Result<[i32; 2]> {
            Ok([
                fixed_pixel(o.x + o.width / 2.)?,
                fixed_pixel(o.y + o.height / 2.)?,
            ])
        };
        let spawn = objects
            .iter()
            .find(|o| o.class == "spawn")
            .map(point)
            .transpose()?
            .ok_or("missing spawn marker")?;
        let beacons = objects
            .iter()
            .filter(|o| o.class == "beacon")
            .map(|o| point(o).map(|point| (o.id, point)))
            .collect::<Result<Vec<_>>>()?;
        let exit = objects
            .iter()
            .find(|o| o.class == "exit")
            .map(point)
            .transpose()?
            .ok_or("missing exit marker")?;
        if beacons.is_empty() {
            return Err("quest needs at least one beacon".into());
        }
        let hash = runtime
            .map()
            .canonical_json()?
            .into_iter()
            .fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
        let world = Self {
            runtime,
            root,
            ground,
            collision,
            markers,
            spawn,
            beacons,
            exit,
            hash,
        };
        for point in std::iter::once(spawn)
            .chain(std::iter::once(world.exit))
            .chain(world.beacons.iter().map(|(_, p)| *p))
        {
            if world.blocked(point)? {
                return Err("required quest marker is blocked or outside the map".into());
            }
        }
        if world.blocked(spawn)? {
            return Err("spawn is blocked".into());
        }
        Ok(world)
    }
    pub fn initial(&self) -> State {
        State {
            position: self.spawn,
            facing_left: false,
            moving: false,
            play_ticks: 0,
            collected: BTreeSet::new(),
            phase: Phase::Playing,
        }
    }
    pub fn blocked(&self, pos: [i32; 2]) -> Result<bool> {
        let x = pos[0] as f64 / 256.;
        let y = pos[1] as f64 / 256.;
        let map = self.runtime.map();
        if x < 5. || y < 5. || x + 5. > map.width as f64 * 16. || y + 5. > map.height as f64 * 16. {
            return Ok(true);
        }
        let bounds = Bounds {
            x: x - 5.,
            y: y - 5.,
            width: 10.,
            height: 10.,
        };
        Ok(self.runtime.collides(self.ground, bounds, "solid")?
            || self.runtime.collides(self.collision, bounds, "solid")?)
    }
    pub fn tick(&self, state: &mut State, input: Input) -> Result {
        if !(-1..=1).contains(&input.x) || !(-1..=1).contains(&input.y) {
            return Err("invalid movement input".into());
        }
        if input.restart {
            *state = self.initial();
            return Ok(());
        }
        if input.pause {
            state.phase = match state.phase {
                Phase::Playing => Phase::Paused,
                Phase::Paused => Phase::Playing,
                p => p,
            };
        }
        state.moving = false;
        if state.phase != Phase::Playing {
            return Ok(());
        }
        state.play_ticks += 1;
        let speed = if input.x != 0 && input.y != 0 {
            181
        } else {
            256
        };
        let delta = [i32::from(input.x) * speed, i32::from(input.y) * speed];
        if input.x != 0 {
            state.facing_left = input.x < 0;
        }
        for (axis, amount) in delta.into_iter().enumerate() {
            let mut next = state.position;
            next[axis] += amount;
            if !self.blocked(next)? {
                state.moving |= next != state.position;
                state.position = next;
            }
        }
        for &(id, pos) in &self.beacons {
            if near(state.position, pos) {
                state.collected.insert(id);
            }
        }
        if state.collected.len() == self.beacons.len() && near(state.position, self.exit) {
            state.phase = Phase::Won;
        }
        Ok(())
    }
    pub fn camera(&self, state: &State) -> View {
        let map = self.runtime.map();
        View {
            x: (state.position[0] as f64 / 256. - WIDTH as f64 / 2.)
                .clamp(0., (map.width as f64 * 16. - WIDTH as f64).max(0.)),
            y: (state.position[1] as f64 / 256. - HEIGHT as f64 / 2.)
                .clamp(0., (map.height as f64 * 16. - HEIGHT as f64).max(0.)),
            width: WIDTH,
            height: HEIGHT,
            zoom: 1.,
        }
    }
}
fn near(a: [i32; 2], b: [i32; 2]) -> bool {
    a[0].abs_diff(b[0]) <= 10 * 256 && a[1].abs_diff(b[1]) <= 10 * 256
}
/// Converts a world-space pixel coordinate to the fixed-point units used by the
/// simulation. Object transforms are validated to `|v| <= 1e9`, so multiplying
/// by 256 in `i32` would overflow; reject anything outside the representable range.
fn fixed_pixel(value: f64) -> Result<i32> {
    let scaled = (value * 256.).round();
    if !scaled.is_finite() || scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err("object coordinates exceed fixed-point range".into());
    }
    Ok(scaled as i32)
}
pub struct Art {
    pub textures: Textures,
    pub tiles: LoadedTilesets,
    pub player: SpriteAtlas,
}
impl Art {
    pub fn load(world: &World) -> Result<Self> {
        let mut textures = Textures::default();
        let mut server = quasar_assets::AssetServer::new();
        server.register(quasar_render_2d::PngLoader);
        server.mount(
            "",
            std::sync::Arc::new(quasar_assets::FileSource::new(world.root.path())?),
        )?;
        let mut images = std::collections::BTreeMap::new();
        for set in &world.runtime.map().tilesets {
            if !images.contains_key(&set.image) {
                let asset = server.load::<quasar_render_2d::PngLoader>(&set.image)?;
                images.insert(set.image.clone(), textures.insert((*asset).clone()));
            }
        }
        let tiles = LoadedTilesets::from_textures(world.runtime.map(), &textures, |path| {
            images
                .get(path)
                .copied()
                .ok_or_else(|| quasar_tilemap::Error {
                    code: "atlas",
                    message: format!("missing preloaded {path}"),
                })
        })?;
        let player = tiles.atlas(0).ok_or("missing player atlas")?.clone();
        Ok(Self {
            textures,
            tiles,
            player,
        })
    }
}
pub fn scene(
    world: &World,
    state: &State,
    art: &Art,
    filter: Filter,
) -> Result<(Canvas, DrawStats)> {
    let view = world.camera(state);
    let mut canvas = Canvas::new(WIDTH, HEIGHT)?;
    canvas.clear = Color(world.runtime.map().background);
    let mut stats = DrawStats::default();
    let elapsed = state.play_ticks * 1000 / 60;
    for layer in &world.runtime.map().layers {
        match &layer.content {
            LayerContent::Tiles { .. } => {
                stats += art.tiles.draw_layer(
                    &world.runtime,
                    layer.id,
                    &mut canvas,
                    view,
                    elapsed,
                    filter,
                )?
            }
            LayerContent::Objects { .. } if layer.id == world.markers => {
                let saved = canvas.camera();
                canvas.set_camera(Camera2D {
                    center: [
                        (view.x + WIDTH as f64 / 2.) as f32,
                        (view.y + HEIGHT as f64 / 2.) as f32,
                    ],
                    zoom: 1.,
                    rotation: 0.,
                });
                for &(id, pos) in &world.beacons {
                    let sprite = art.player.sprite(
                        if state.collected.contains(&id) {
                            17
                        } else {
                            16
                        },
                        Rect::new(
                            pos[0] as f32 / 256. - 8.,
                            pos[1] as f32 / 256. - 10.,
                            16.,
                            16.,
                        ),
                    )?;
                    canvas.sprite(sprite);
                }
                canvas.sprite(art.player.sprite(
                    if state.collected.len() == world.beacons.len() {
                        19
                    } else {
                        18
                    },
                    Rect::new(
                        world.exit[0] as f32 / 256. - 8.,
                        world.exit[1] as f32 / 256. - 8.,
                        16.,
                        16.,
                    ),
                )?);
                let player_frame = if state.moving {
                    world
                        .runtime
                        .animated_tile(0, 24, elapsed)
                        .expect("validated player atlas")
                } else {
                    24
                };
                canvas.sprite(art.player.sprite(
                    30,
                    Rect::new(
                        state.position[0] as f32 / 256. - 8.,
                        state.position[1] as f32 / 256. - 7.,
                        16.,
                        16.,
                    ),
                )?);
                canvas.sprite_flipped(
                    art.player.sprite(
                        player_frame,
                        Rect::new(
                            state.position[0] as f32 / 256. - 8.,
                            state.position[1] as f32 / 256. - 11.,
                            16.,
                            16.,
                        ),
                    )?,
                    state.facing_left,
                    false,
                );
                canvas.set_camera(saved);
            }
            _ => {}
        }
    }
    canvas.rect(Rect::new(8., 8., 464., 32.), [10, 28, 28, 235]);
    canvas.rect(Rect::new(8., 39., 464., 1.), [75, 117, 89, 255]);
    canvas.text([18., 17.], "TILEQUEST", 2., [234, 218, 162, 255]);
    canvas.text(
        [290., 15.],
        &format!("BEACONS {}/{}", state.collected.len(), world.beacons.len()),
        1.,
        [242, 189, 90, 255],
    );
    canvas.text([290., 28.], "FIND THE OLD GATE", 1., [146, 185, 159, 255]);
    canvas.rect(Rect::new(8., 296., 464., 16.), [10, 28, 28, 235]);
    canvas.text(
        [18., 301.],
        "WASD / ARROWS MOVE   P PAUSE   R RESTART   Q QUIT",
        1.,
        [168, 193, 164, 255],
    );
    if state.phase != Phase::Playing {
        canvas.rect(Rect::new(108., 125., 264., 65.), [10, 28, 28, 245]);
        canvas.text(
            [134., 140.],
            if state.phase == Phase::Won {
                "GATE RESTORED"
            } else {
                "PAUSED"
            },
            2.,
            [242, 205, 124, 255],
        );
        canvas.text(
            [134., 170.],
            if state.phase == Phase::Won {
                "R TO EXPLORE AGAIN"
            } else {
                "P TO RESUME"
            },
            1.,
            [165, 194, 170, 255],
        );
    }
    Ok((canvas, stats))
}
#[derive(Default)]
pub struct Autoplay {
    route: VecDeque<[i32; 2]>,
    goal: Option<[i32; 2]>,
}
impl Autoplay {
    pub fn input(&mut self, world: &World, state: &State) -> Result<Input> {
        if state.phase != Phase::Playing {
            return Ok(Input::default());
        }
        let target = world
            .beacons
            .iter()
            .find(|(id, _)| !state.collected.contains(id))
            .map(|(_, p)| *p)
            .unwrap_or(world.exit);
        if self.goal != Some(target) || self.route.is_empty() {
            self.route = find_path(world, state.position, target)?;
            self.goal = Some(target);
        }
        while self.route.front().is_some_and(|p| {
            p[0].abs_diff(state.position[0]) < 256 && p[1].abs_diff(state.position[1]) < 256
        }) {
            self.route.pop_front();
        }
        let point = self.route.front().copied().unwrap_or(target);
        let x = (point[0] - state.position[0]).signum() as i8;
        let y = (point[1] - state.position[1]).signum() as i8;
        Ok(if point[0].abs_diff(state.position[0]) >= 256 {
            Input {
                x,
                y: 0,
                ..Default::default()
            }
        } else {
            Input {
                x: 0,
                y,
                ..Default::default()
            }
        })
    }
}
fn find_path(world: &World, start: [i32; 2], target: [i32; 2]) -> Result<VecDeque<[i32; 2]>> {
    let map = world.runtime.map();
    let width = map.width as usize;
    let count = width * map.height as usize;
    if count > quasar_tilemap::MAX_CELLS {
        return Err("quest navigation grid exceeds budget".into());
    }
    let cell = |p: [i32; 2]| (p[1] / 4096) as usize * width + (p[0] / 4096) as usize;
    let first = cell(start);
    let last = cell(target);
    let mut parent = vec![usize::MAX; count];
    let mut queue = VecDeque::from([first]);
    parent[first] = first;
    while let Some(index) = queue.pop_front() {
        if index == last {
            break;
        }
        let x = (index % width) as i32;
        let y = (index / width) as i32;
        for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= map.width as i32 || ny >= map.height as i32 {
                continue;
            }
            let next = ny as usize * width + nx as usize;
            if parent[next] == usize::MAX && !world.blocked([nx * 4096 + 2048, ny * 4096 + 2048])? {
                parent[next] = index;
                queue.push_back(next);
            }
        }
    }
    if parent[last] == usize::MAX {
        return Err("quest target is unreachable".into());
    }
    let mut route = VecDeque::new();
    let mut at = last;
    while at != first {
        route.push_front([
            (at % width) as i32 * 4096 + 2048,
            (at / width) as i32 * 4096 + 2048,
        ]);
        at = parent[at];
    }
    Ok(route)
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    pub version: u32,
    pub map_hash: u64,
    pub inputs: Vec<Input>,
}
impl Replay {
    pub fn validate(&self, world: &World) -> Result {
        if self.version != 1
            || self.map_hash != world.hash
            || self.inputs.len() > 1_000_000
            || self
                .inputs
                .iter()
                .any(|i| !(-1..=1).contains(&i.x) || !(-1..=1).contains(&i.y))
        {
            return Err("invalid replay version, map fingerprint, length or input".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
