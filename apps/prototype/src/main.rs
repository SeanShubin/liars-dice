//! The die rendering prototype. One die, a ground, and live appearance settings.
//!
//! Composition root. No behavior: the mesh, the textures, and the materials all come
//! from dice-render. What lives here is the window, the camera, the settings panel, and
//! the wiring between them.
//!
//! Realizes /specification/die-rendering-prototype.md.

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use dice_render::palette;
use dice_render::settings::{Glyph, Settings, Surface};
use dice_render::texture::{BEVEL_PIXELS, GROUND_PIXELS};
use dice_render::{
    bevel_mesh, face_base_color, face_normal_map, faces_mesh, surface_base_color,
    surface_normal_map,
};

/// The settings the panel edits.
#[derive(Resource, Clone, Copy)]
struct Appearance(Settings);

/// The seed the current appearance was generated from, when it was generated rather
/// than hand-edited. Shown so an appearance can be returned to.
#[derive(Resource, Default)]
struct PaletteSeed(u32);

/// What the panel last reported about the pointer, so the camera does not fight it.
#[derive(Resource, Default)]
struct PanelHasPointer(bool);

/// Where the camera is, in spherical coordinates about the die.
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

/// Handles to everything regenerated when a setting changes, and the settings they were
/// generated from.
#[derive(Resource)]
struct Built {
    faces: Handle<Mesh>,
    corners: Handle<Mesh>,
    face_color: Handle<Image>,
    face_normal: Handle<Image>,
    bevel_color: Handle<Image>,
    bevel_normal: Handle<Image>,
    ground_color: Handle<Image>,
    ground_normal: Handle<Image>,
    face_material: Handle<StandardMaterial>,
    bevel_material: Handle<StandardMaterial>,
    ground_material: Handle<StandardMaterial>,
    from: Settings,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Liars Dice — die rendering prototype".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(Appearance(Settings::default()))
        .insert_resource(PanelHasPointer::default())
        .insert_resource(PaletteSeed::default())
        .insert_resource(Orbit {
            yaw: 0.6,
            pitch: 0.45,
            distance: 6.0,
        })
        .add_systems(Startup, setup)
        .add_systems(EguiPrimaryContextPass, panel)
        .add_systems(Update, (rebuild, orbit))
        .run();
}

fn material(color: Handle<Image>, normal: Handle<Image>, surface: &Surface) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(color),
        normal_map_texture: Some(normal),
        perceptual_roughness: surface.roughness.clamp(0.02, 1.0),
        metallic: surface.metallic.clamp(0.0, 1.0),
        ..default()
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    appearance: Res<Appearance>,
) {
    let set = appearance.0;

    let faces = meshes.add(faces_mesh(&set.shape));
    let corners = meshes.add(bevel_mesh(&set.shape));

    let face_color = images.add(face_base_color(&set.face, &set.glyph));
    let face_normal = images.add(face_normal_map(&set.face, &set.glyph));
    let bevel_color = images.add(surface_base_color(&set.bevel, BEVEL_PIXELS));
    let bevel_normal = images.add(surface_normal_map(&set.bevel, BEVEL_PIXELS));
    let ground_color = images.add(surface_base_color(&set.ground, GROUND_PIXELS));
    let ground_normal = images.add(surface_normal_map(&set.ground, GROUND_PIXELS));

    let face_material = materials.add(material(face_color.clone(), face_normal.clone(), &set.face));
    let bevel_material = materials.add(material(
        bevel_color.clone(),
        bevel_normal.clone(),
        &set.bevel,
    ));
    let ground_material = materials.add(material(
        ground_color.clone(),
        ground_normal.clone(),
        &set.ground,
    ));

    commands.spawn((Mesh3d(faces.clone()), MeshMaterial3d(face_material.clone())));
    commands.spawn((
        Mesh3d(corners.clone()),
        MeshMaterial3d(bevel_material.clone()),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(ground_material.clone()),
        Transform::from_xyz(0.0, -1.0, 0.0),
    ));

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(5.0, 9.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 3.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.insert_resource(Built {
        faces,
        corners,
        face_color,
        face_normal,
        bevel_color,
        bevel_normal,
        ground_color,
        ground_normal,
        face_material,
        bevel_material,
        ground_material,
        from: set,
    });
}

fn surface_controls(ui: &mut egui::Ui, surface: &mut Surface) {
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut surface.color);
        ui.label("colour");
    });
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut surface.speckle_color);
        ui.label("speckle colour");
    });
    ui.add(egui::Slider::new(&mut surface.speckle_amount, 0.0..=1.0).text("speckles"));
    ui.add(egui::Slider::new(&mut surface.speckle_scale, 2.0..=300.0).text("speckle scale"));
    ui.add(egui::Slider::new(&mut surface.bump_strength, 0.0..=1.0).text("roughness relief"));
    ui.add(egui::Slider::new(&mut surface.roughness, 0.02..=1.0).text("flatness"));
    ui.add(egui::Slider::new(&mut surface.metallic, 0.0..=1.0).text("metallic"));
    ui.add(egui::Slider::new(&mut surface.seed, 0..=64).text("seed"));
}

fn glyph_controls(ui: &mut egui::Ui, glyph: &mut Glyph) {
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut glyph.color);
        ui.label("pip colour");
    });
    ui.add(egui::Slider::new(&mut glyph.radius, 0.05..=0.4).text("pip radius"));
    ui.add(egui::Slider::new(&mut glyph.depth, 0.0..=2.0).text("pip depth"));
}

fn panel(
    mut contexts: EguiContexts,
    mut appearance: ResMut<Appearance>,
    mut pointer: ResMut<PanelHasPointer>,
    mut seed: ResMut<PaletteSeed>,
) -> Result {
    let context = contexts.ctx_mut()?;
    egui::Window::new("Appearance")
        .default_width(320.0)
        .default_pos([16.0, 16.0])
        .show(context, |ui| {
            ui.label("Drag outside this window to rotate. Scroll to zoom.");

            let set = &mut appearance.0;

            ui.horizontal(|ui| {
                if ui.button("Random").clicked() {
                    seed.0 = seed.0.wrapping_add(1);
                    palette::random(seed.0, set);
                }
                if ui
                    .add(egui::DragValue::new(&mut seed.0).prefix("seed "))
                    .changed()
                {
                    palette::random(seed.0, set);
                }
                if ui.button("Reset").clicked() {
                    *set = Settings::default();
                }
            });
            ui.separator();

            ui.collapsing("Shape", |ui| {
                ui.add(
                    egui::Slider::new(
                        &mut set.shape.rounding,
                        dice_render::settings::MIN_ROUNDING..=dice_render::settings::MAX_ROUNDING,
                    )
                    .text("rounding"),
                );
                ui.add(egui::Slider::new(&mut set.shape.arc_segments, 1..=32).text("arc segments"));
                ui.add(egui::Slider::new(&mut set.shape.corner_rings, 1..=12).text("corner rings"));
            });

            ui.collapsing("Face", |ui| surface_controls(ui, &mut set.face));
            ui.collapsing("Pips", |ui| glyph_controls(ui, &mut set.glyph));
            ui.collapsing("Bevel", |ui| surface_controls(ui, &mut set.bevel));
            ui.collapsing("Ground", |ui| surface_controls(ui, &mut set.ground));
        });
    pointer.0 = context.is_pointer_over_egui();
    Ok(())
}

fn rebuild(
    appearance: Res<Appearance>,
    mut built: ResMut<Built>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let set = appearance.0;
    if set == built.from {
        return;
    }

    if set.shape != built.from.shape {
        if let Some(mut mesh) = meshes.get_mut(&built.faces) {
            *mesh = faces_mesh(&set.shape);
        }
        if let Some(mut mesh) = meshes.get_mut(&built.corners) {
            *mesh = bevel_mesh(&set.shape);
        }
    }

    if set.face != built.from.face || set.glyph != built.from.glyph {
        if let Some(mut image) = images.get_mut(&built.face_color) {
            *image = face_base_color(&set.face, &set.glyph);
        }
        if let Some(mut image) = images.get_mut(&built.face_normal) {
            *image = face_normal_map(&set.face, &set.glyph);
        }
    }

    if set.bevel != built.from.bevel {
        if let Some(mut image) = images.get_mut(&built.bevel_color) {
            *image = surface_base_color(&set.bevel, BEVEL_PIXELS);
        }
        if let Some(mut image) = images.get_mut(&built.bevel_normal) {
            *image = surface_normal_map(&set.bevel, BEVEL_PIXELS);
        }
    }

    if set.ground != built.from.ground {
        if let Some(mut image) = images.get_mut(&built.ground_color) {
            *image = surface_base_color(&set.ground, GROUND_PIXELS);
        }
        if let Some(mut image) = images.get_mut(&built.ground_normal) {
            *image = surface_normal_map(&set.ground, GROUND_PIXELS);
        }
    }

    for (handle, surface) in [
        (&built.face_material, &set.face),
        (&built.bevel_material, &set.bevel),
        (&built.ground_material, &set.ground),
    ] {
        if let Some(mut material) = materials.get_mut(handle) {
            material.perceptual_roughness = surface.roughness.clamp(0.02, 1.0);
            material.metallic = surface.metallic.clamp(0.0, 1.0);
        }
    }

    built.from = set;
}

fn orbit(
    mut orbit: ResMut<Orbit>,
    pointer: Res<PanelHasPointer>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let mut dragged = Vec2::ZERO;
    for event in motion.read() {
        dragged += event.delta;
    }
    let mut scrolled = 0.0;
    for event in wheel.read() {
        scrolled += event.y;
    }

    if pointer.0 {
        return;
    }

    if buttons.pressed(MouseButton::Left) {
        orbit.yaw -= dragged.x * 0.006;
        orbit.pitch = (orbit.pitch + dragged.y * 0.006).clamp(-1.5, 1.5);
    }
    if scrolled != 0.0 {
        orbit.distance = (orbit.distance - scrolled * 0.4).clamp(2.2, 24.0);
    }

    let direction = Vec3::new(
        orbit.pitch.cos() * orbit.yaw.sin(),
        orbit.pitch.sin(),
        orbit.pitch.cos() * orbit.yaw.cos(),
    );
    for mut transform in &mut cameras {
        *transform =
            Transform::from_translation(direction * orbit.distance).looking_at(Vec3::ZERO, Vec3::Y);
    }
}
