use std::{collections::BTreeMap, path::Path};

use bevy::{asset::Handle, math::Vec3};
use bevy_flash::{
    sampling::VabSkin,
    vab_asset::{MeshMaterial, RenderMeshGroup, VabAsset, VabCommand},
};
use vatf::{
    animation::{AnimMatrix, AnimTransform},
    baked::{BakedClip, BakedMovie, BakedNode, BakedSkin, BakedSkinVariant},
    reader::VabReader,
};

fn collect_skin_slots(nodes: &[BakedNode], slots: &mut BTreeMap<String, u16>) {
    for node in nodes {
        match node {
            BakedNode::Skin { slot, symbol, .. } => {
                if let Some(previous) = slots.insert(slot.clone(), *symbol) {
                    assert_eq!(previous, *symbol, "skin slot changed symbol");
                }
            }
            BakedNode::Group { children, .. } => collect_skin_slots(children, slots),
            BakedNode::Mask { mask, children } => {
                collect_skin_slots(mask, slots);
                collect_skin_slots(children, slots);
            }
            BakedNode::Shape { .. } => {}
        }
    }
}

#[test]
fn skin_selection_preserves_slot_transform_and_is_instance_local() {
    let mut asset = VabAsset {
        shape_map: Default::default(),
        morph_map: Default::default(),
        render_meshes: vec![
            RenderMeshGroup {
                mesh: Handle::default(),
                material: MeshMaterial::Color,
                local_bounds: [0.0, 0.0, 10.0, 10.0]
            };
            2
        ],
        baked: BakedMovie {
            frame_rate: 30.0,
            clips: vec![BakedClip {
                name: "idle".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![vec![BakedNode::Skin {
                    slot: "hand".into(),
                    symbol: 10,
                    transform: AnimTransform {
                        matrix: AnimMatrix {
                            tx: 20.0,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                }]],
            }],
            skins: vec![BakedSkin {
                symbol: 10,
                variants: vec![
                    BakedSkinVariant {
                        name: "default".into(),
                        nodes: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: Default::default(),
                        }],
                    },
                    BakedSkinVariant {
                        name: "red_armor".into(),
                        nodes: vec![BakedNode::Shape {
                            id: 2,
                            ratio: 0,
                            transform: Default::default(),
                        }],
                    },
                ],
            }],
        },
    };
    asset.shape_map.insert(1, vec![0]);
    asset.shape_map.insert(2, vec![1]);
    let original = VabSkin::default();
    let mut changed = original.clone();
    changed.set(&asset, "hand", "red_armor").unwrap();
    assert!(changed.set(&asset, "hand", "missing").is_err());
    assert!(
        changed
            .set_many(&asset, [("hand", "default"), ("missing", "default")])
            .is_err()
    );
    assert_eq!(changed.selection("hand"), Some("red_armor"));
    assert_eq!(
        asset.skin_variant_names("hand").unwrap(),
        ["default", "red_armor"]
    );
    assert_eq!(asset.skin_slots(), ["hand"]);
    for (skin, expected) in [(&original, 0), (&changed, 1)] {
        let commands = asset.sample(0, 0, skin, Vec3::ONE).unwrap();
        match &commands.commands[0] {
            VabCommand::RenderShape { handle, transform } => {
                assert_eq!(*handle, expected);
                assert_eq!(transform.matrix.tx, 20.0);
            }
            _ => panic!("expected selected shape"),
        }
    }
    let scaled = asset.sample(0, 0, &changed, Vec3::splat(2.0)).unwrap();
    match &scaled.commands[0] {
        VabCommand::RenderShape { transform, .. } => {
            assert_eq!(transform.matrix.tx, 40.0);
            assert_eq!(transform.matrix.a, 2.0);
        }
        _ => panic!("expected scaled skin"),
    }
}

#[test]
fn wu_kong_fixture_preserves_direct_skin_frame_labels() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/wu_kong.vab");
    let reader = VabReader::open(&path).expect("checked-in wu_kong.vab must be readable");
    let movie = reader.baked();
    assert!(!movie.skins.is_empty(), "wu_kong must contain baked skins");

    let mut slots = BTreeMap::new();
    for clip in &movie.clips {
        for frame in &clip.frames {
            collect_skin_slots(frame, &mut slots);
        }
    }
    assert!(
        !slots.is_empty(),
        "wu_kong must reference at least one skin slot"
    );

    for (slot, symbol) in slots {
        let skin = movie
            .skins
            .iter()
            .find(|skin| skin.symbol == symbol)
            .unwrap_or_else(|| panic!("skin slot {slot} references missing symbol {symbol}"));
        assert!(
            skin.variants.iter().any(|variant| variant.name == "skin_1"),
            "skin slot {slot} must retain the source label skin_1"
        );
    }
}
