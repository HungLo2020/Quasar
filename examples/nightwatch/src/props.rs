//! Authoritative game-owned prop/character definitions. No renderer or scene graph.
use quasar_physics::dim3::{PlacedShape3, RigidPose3, Shape3};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelId {
    Pistol,
    Rifle,
    CrateMedium,
    CrateWide,
    Barrel,
    Workbench,
    MetalPanel,
}
impl ModelId {
    pub const ALL: [Self; 7] = [
        Self::Pistol,
        Self::Rifle,
        Self::CrateMedium,
        Self::CrateWide,
        Self::Barrel,
        Self::Workbench,
        Self::MetalPanel,
    ];
    pub fn file(self) -> &'static str {
        match self {
            Self::Pistol => "blaster-kit/blaster-d.glb",
            Self::Rifle => "blaster-kit/blaster-f.glb",
            Self::CrateMedium => "blaster-kit/crate-medium.glb",
            Self::CrateWide => "blaster-kit/crate-wide.glb",
            Self::Barrel => "survival-kit/barrel.glb",
            Self::Workbench => "survival-kit/workbench.glb",
            Self::MetalPanel => "survival-kit/metal-panel.glb",
        }
    }
    pub fn is_weapon(self) -> bool {
        matches!(self, Self::Pistol | Self::Rifle)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum PropVisual {
    Model {
        asset: ModelId,
        half: [f32; 3],
    },
    Cuboid {
        half: [f32; 3],
        color: [f32; 3],
        emissive: bool,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub id: u64,
    pub name: String,
    pub pose: RigidPose3,
    pub visual: PropVisual,
    /// None is an explicit decorative/nonblocking choice, never inferred from the mesh.
    pub collider: Option<Shape3>,
}
impl Prop {
    pub fn cuboid(
        id: u64,
        name: impl Into<String>,
        center: [f32; 3],
        half: [f32; 3],
        color: [f32; 3],
    ) -> Self {
        Self {
            id,
            name: name.into(),
            pose: RigidPose3::translated(center),
            visual: PropVisual::Cuboid {
                half,
                color,
                emissive: false,
            },
            collider: Some(Shape3::Cuboid { half }),
        }
    }
    pub fn model(
        id: u64,
        name: impl Into<String>,
        center: [f32; 3],
        half: [f32; 3],
        asset: ModelId,
        shape: Shape3,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            pose: RigidPose3::translated(center),
            visual: PropVisual::Model { asset, half },
            collider: Some(shape),
        }
    }
    pub fn placed_collider(&self) -> Option<PlacedShape3> {
        self.collider.clone().map(|shape| PlacedShape3 {
            pose: self.pose,
            shape,
        })
    }
}
pub const PLAYER_HEIGHT: f32 = 1.85;
pub const PLAYER_RADIUS: f32 = 0.32;
pub const ZOMBIE_HEIGHT: f32 = 1.94;
pub const ZOMBIE_RADIUS: f32 = 0.34;
pub const SKIN: f32 = 0.005;
/// Explicit cuboid approximation of the workbench top and its four tapered legs.
/// Ratios come from the structural `workbench` node, excluding loose tools/paper.
pub fn workbench_shape(half: [f32; 3]) -> Shape3 {
    let leg_center = 0.10695 / 0.14782624;
    let leg_half = 0.02325 / 0.14782624;
    let top_bottom = 0.1953 / 0.23655 * 2. - 1.;
    let mut parts = vec![PlacedShape3 {
        pose: RigidPose3::translated([0., (1. + top_bottom) * 0.5 * half[1], 0.]),
        shape: Shape3::Cuboid {
            half: [half[0], (1. - top_bottom) * 0.5 * half[1], half[2]],
        },
    }];
    for x in [-1., 1.] {
        for z in [-1., 1.] {
            parts.push(PlacedShape3 {
                pose: RigidPose3::translated([
                    x * leg_center * half[0],
                    (top_bottom - 1.) * 0.5 * half[1],
                    z * leg_center * half[2],
                ]),
                shape: Shape3::Cuboid {
                    half: [
                        leg_half * half[0],
                        (top_bottom + 1.) * 0.5 * half[1],
                        leg_half * half[2],
                    ],
                },
            });
        }
    }
    Shape3::Compound(parts)
}
pub fn courtyard() -> Vec<Prop> {
    let mut p = vec![
        Prop::cuboid(
            1,
            "floor",
            [0., -0.25, 0.],
            [15.5, 0.25, 15.5],
            [0.095, 0.105, 0.115],
        ),
        Prop::model(
            100,
            "west crate",
            [-5., 1.15, -2.],
            [1.4, 1.15, 2.],
            ModelId::CrateMedium,
            Shape3::Cuboid {
                half: [1.4, 1.15, 2.],
            },
        ),
        Prop::model(
            101,
            "east wide crate",
            [4.5, 0.8, -4.5],
            [2.3, 0.8, 1.],
            ModelId::CrateWide,
            Shape3::Cuboid {
                half: [2.3, 0.8, 1.],
            },
        ),
        Prop::model(
            102,
            "east barricade",
            [7.5, 1.4, 3.],
            [0.85, 1.4, 2.5],
            ModelId::CrateMedium,
            Shape3::Cuboid {
                half: [0.85, 1.4, 2.5],
            },
        ),
        Prop::model(
            103,
            "north crate",
            [10., 1., -9.],
            [1.5, 1., 1.5],
            ModelId::CrateMedium,
            Shape3::Cuboid {
                half: [1.5, 1., 1.5],
            },
        ),
        Prop::model(
            104,
            "workbench",
            [0., 0.6, -0.5],
            [1.6, 0.6, 0.65],
            ModelId::Workbench,
            workbench_shape([1.6, 0.6, 0.65]),
        ),
    ];
    for (group, x, z, half_height, separation) in [(0, -4., 5., 0.7, 0.7), (1, -9., -8., 0.8, 0.6)]
    {
        // Preserve the actual previous uniform model scale, not the old oversized box.
        let visual_radius = half_height * 2. * 0.11825 / 0.344;
        let radius = half_height * 2. * 0.13364123 / 0.344;
        for (side, sign) in [(0, -1.), (1, 1.)] {
            p.push(Prop::model(
                200 + group * 2 + side,
                format!("barrel {group}/{side}"),
                [x + sign * separation, half_height, z],
                [visual_radius, half_height, visual_radius],
                ModelId::Barrel,
                Shape3::CylinderY {
                    half_height,
                    radius,
                },
            ));
        }
    }
    for (i, sign) in [(0, -1.), (1, 1.)] {
        p.push(Prop::cuboid(
            300 + i,
            "side wall",
            [sign * 15., 2.5, 0.],
            [0.5, 2.5, 15.],
            [0.12, 0.16, 0.19],
        ));
        p.push(Prop::cuboid(
            302 + i,
            "end wall",
            [0., 2.5, sign * 15.],
            [15., 2.5, 0.5],
            [0.13, 0.17, 0.20],
        ));
        p.push(Prop::cuboid(
            304 + i,
            "wall rail",
            [sign * 14.25, 4.5, 0.],
            [0.22, 0.14, 14.],
            [0.22, 0.25, 0.28],
        ));
        for (j, z) in (-3..4).enumerate() {
            p.push(Prop::cuboid(
                320 + i * 8 + j as u64,
                "wall post",
                [sign * 14.35, 2.6, z as f32 * 4.],
                [0.18, 2.6, 0.25],
                [0.19, 0.24, 0.28],
            ));
        }
    }
    p.push(Prop::cuboid(
        400,
        "gate panel",
        [0., 2., -14.4],
        [3., 2., 0.2],
        [0.1, 0.13, 0.15],
    ));
    for (i, x) in (-7..8).enumerate() {
        p.push(Prop::cuboid(
            410 + i as u64,
            "gate bar",
            [x as f32 * 0.38, 2., -14.15],
            [0.025, 1.8, 0.035],
            [0.22, 0.25, 0.26],
        ));
    }
    let mut sign = Prop::cuboid(
        450,
        "gate light",
        [0., 4.3, -14.1],
        [1.8, 0.10, 0.06],
        [0.8, 0.08, 0.06],
    );
    if let PropVisual::Cuboid { emissive, .. } = &mut sign.visual {
        *emissive = true;
    }
    p.push(sign);
    for (i, z) in [-10., -4., 2., 8.].into_iter().enumerate() {
        p.push(Prop::model(
            500 + i as u64,
            "metal wall panel",
            [14.3, 2.2, z],
            [0.035, 1., 1.2],
            ModelId::MetalPanel,
            Shape3::Cuboid {
                half: [0.035, 1., 1.2],
            },
        ));
    }
    for (i, (position, color)) in [
        ([-9., 3., -7.], [0.1, 0.7, 1.]),
        ([8., 3., -5.], [1., 0.3, 0.08]),
        ([-8., 3., 8.], [0.2, 0.55, 1.]),
        ([9., 3., 9.], [1., 0.65, 0.25]),
    ]
    .into_iter()
    .enumerate()
    {
        p.push(Prop::cuboid(
            600 + i as u64 * 3,
            "lamp post",
            [position[0], 1.5, position[2]],
            [0.08, 1.5, 0.08],
            [0.16, 0.2, 0.22],
        ));
        p.push(Prop::cuboid(
            601 + i as u64 * 3,
            "lamp housing",
            [position[0], 3.1, position[2]],
            [0.5, 0.12, 0.22],
            [0.08, 0.1, 0.12],
        ));
        let mut bulb = Prop::cuboid(
            602 + i as u64 * 3,
            "lamp glass",
            position,
            [0.4, 0.05, 0.18],
            color,
        );
        if let PropVisual::Cuboid { emissive, .. } = &mut bulb.visual {
            *emissive = true;
        }
        p.push(bulb);
    }
    p
}
