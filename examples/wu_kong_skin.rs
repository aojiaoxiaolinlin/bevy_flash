use bevy::prelude::*;
use bevy_flash::{
    FlashPlayerPlugin,
    sampling::VabSkin,
    vab_asset::{VabAsset, VabAssetHandle},
    vab_player::VabPlayer,
};

#[derive(Component)]
struct WuKong;

#[derive(Component)]
struct StartWhenLoaded;

#[derive(Resource, Default)]
struct SkinCatalog {
    slots: Vec<String>,
    variants: Vec<String>,
}

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(102, 102, 102)))
        .init_resource::<SkinCatalog>()
        .add_plugins((DefaultPlugins, FlashPlayerPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, (start_when_loaded, keyboard_control))
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((Camera2d, CompositingSpace::Srgb));
    commands.spawn((
        Name::new("wu_kong"),
        WuKong,
        StartWhenLoaded,
        VabAssetHandle(asset_server.load("wu_kong.vab")),
        VabPlayer::default(),
        VabSkin::default(),
        // The source occupies roughly x=331..407, y=63..179. Scale around
        // its center so the small authoring asset is easy to inspect.
        Transform {
            translation: Vec3::new(-369.2 * 4.0, 120.8 * 4.0, 0.0),
            scale: Vec3::splat(4.0),
            ..default()
        },
    ));
    info!("controls: 1-9 = select a named skin variant listed after loading");
}

fn start_when_loaded(
    mut commands: Commands,
    assets: Res<Assets<VabAsset>>,
    mut catalog: ResMut<SkinCatalog>,
    mut instances: Query<(Entity, &VabAssetHandle, &mut VabSkin), With<StartWhenLoaded>>,
) {
    for (entity, handle, mut skin) in &mut instances {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        let slots = asset.skin_slots();
        if slots.is_empty() {
            warn!("wu_kong.vab has no skin_<slot> instances; regenerate it after updating the SWF");
            commands.entity(entity).remove::<StartWhenLoaded>();
            continue;
        }
        let mut variants = match asset.skin_variant_names(&slots[0]) {
            Ok(variants) => variants,
            Err(error) => {
                warn!("wu_kong skin metadata is invalid: {error}");
                commands.entity(entity).remove::<StartWhenLoaded>();
                continue;
            }
        };
        variants.retain(|variant| {
            let mut probe = VabSkin::default();
            probe
                .set_many(
                    asset,
                    slots.iter().map(|slot| (slot.as_str(), variant.as_str())),
                )
                .is_ok()
        });
        let Some(initial) = variants.first() else {
            warn!("wu_kong skin slots have no common named variant");
            commands.entity(entity).remove::<StartWhenLoaded>();
            continue;
        };
        if let Err(error) = skin.set_many(
            asset,
            slots.iter().map(|slot| (slot.as_str(), initial.as_str())),
        ) {
            warn!("wu_kong skin metadata is invalid: {error}");
            continue;
        }
        info!("wu_kong slots: {}", slots.join(", "));
        info!("number keys select: {}", variants.join(", "));
        catalog.slots = slots;
        catalog.variants = variants;
        commands.entity(entity).remove::<StartWhenLoaded>();
    }
}

fn keyboard_control(
    keyboard: Res<ButtonInput<KeyCode>>,
    assets: Res<Assets<VabAsset>>,
    catalog: Res<SkinCatalog>,
    mut instances: Query<(&VabAssetHandle, &mut VabSkin), With<WuKong>>,
) {
    let next = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .into_iter()
    .position(|key| keyboard.just_pressed(key));
    let Some(variant) = next.and_then(|index| catalog.variants.get(index)) else {
        return;
    };

    for (handle, mut skin) in &mut instances {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        match skin.set_many(
            asset,
            catalog
                .slots
                .iter()
                .map(|slot| (slot.as_str(), variant.as_str())),
        ) {
            Ok(()) => {
                info!("selected skin variant {variant}");
            }
            Err(error) => warn!("unable to select skin variant {variant}: {error}"),
        }
    }
}
