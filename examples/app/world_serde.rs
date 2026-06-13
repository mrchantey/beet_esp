//! beet `world_serde` on the ESP32 — load a scene from JSON, then run a system.
//!
//! Goal: prove beet's reflection-backed world serialization works in `no_std` on
//! hardware. The eventual aim is to ship a scene over the wire and have the
//! device load + run it; this is the smallest end-to-end slice of that.
//!
//! What it does, all in one `Startup` chain:
//! - [`dump_canonical`] spawns a [`HelloWorld`], serializes that subtree to JSON
//!   with [`TemplateSaver`], and logs it — this is the ground-truth wire format.
//! - [`load_scene`] deserializes the hard-coded [`SCENE_JSON`] back into the world
//!   with [`TemplateLoader`].
//! - [`greet`] queries for [`HelloWorld`] and logs "hello world" for each — the
//!   "a system runs when the component is present" half of the demo.
//!
//! No Wi-Fi/LED here; [`Esp32Plugin`] just brings up the chip and ticks the
//! schedule. Types must be reflected and registered for serde to find them.
//!
//! Run with: `cargo run --release --no-default-features --example world_serde`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

/// Marker component. Reflected + registered so `world_serde` can (de)serialize it.
///
/// The `type_path` override pins the serialized key to a stable string,
/// independent of this example's crate name, so [`SCENE_JSON`] stays readable.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
#[type_path = "beet_esp"]
struct HelloWorld;

/// A hard-coded scene: one entity carrying a [`HelloWorld`]. The loader remaps
/// the entity id, but the key must still be valid `Entity::to_bits` — bevy stores
/// the row index inverted, so `4294967295` (`0xFFFFFFFF`) is index 0, generation
/// first. Component values key by reflect type path.
const SCENE_JSON: &str = r#"{
  "resources": {},
  "nodes": {
    "4294967295": {
      "components": {
        "beet_esp::HelloWorld": {}
      }
    }
  }
}"#;

#[beet_esp::main]
fn main() {
    let mut app = App::new();
    app.add_plugins((Esp32Plugin, HealthPlugin));
    // world_serde looks types up by reflection at load time.
    app.init_resource::<AppTypeRegistry>();
    app.register_type::<HelloWorld>();
    app.add_systems(Startup, (dump_canonical, load_scene, greet).chain());
    app.run();
}

/// Serialize a freshly-spawned [`HelloWorld`] subtree to JSON and log it, so the
/// device itself reports the canonical wire format. The temp entity is despawned
/// so it doesn't show up in [`greet`].
fn dump_canonical(world: &mut World) {
    let temp = world.spawn(HelloWorld).id();
    match TemplateSaver::new()
        .with_entity_tree(world, temp)
        .save(world, MediaType::Json)
    {
        Ok(bytes) => match bytes.as_utf8() {
            Ok(text) => info!("canonical scene JSON:\n{}", text),
            Err(e) => warn!("serialized scene not UTF-8: {:?}", e),
        },
        Err(e) => warn!("failed to serialize scene: {:?}", e),
    }
    world.despawn(temp);
}

/// Deserialize [`SCENE_JSON`] into the world via [`TemplateLoader`].
fn load_scene(world: &mut World) {
    let bytes = MediaBytes::new_json(SCENE_JSON);
    match TemplateLoader::new(world).load(&bytes) {
        Ok(spawned) => info!("loaded {} entit(ies) from hard-coded JSON scene", spawned.len()),
        Err(e) => warn!("failed to load scene: {:?}", e),
    }
}

/// The payload system: runs once the scene is loaded and greets each entity that
/// carries the [`HelloWorld`] component.
fn greet(query: Query<Entity, With<HelloWorld>>) {
    let count = query.iter().count();
    info!("{} entit(ies) with HelloWorld present", count);
    for _ in &query {
        info!("hello world");
    }
}
