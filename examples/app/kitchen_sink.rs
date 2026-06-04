//! Everything at once: LED + Wi-Fi (client & server) + `world_serde`, under the
//! health monitor. Stacks the heaviest things we do on one Bevy `World` to gauge
//! headroom; flash it and read the periodic health report.
//!
//! Run with:
//!   `cargo run --release --no-default-features --features led,wifi,action --example kitchen_sink`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::Debug2Format;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_time::Duration;
use embassy_time::Timer;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
#[type_path = "beet_esp"]
struct HelloWorld;

const SCENE_JSON: &str = r#"{
  "resources": {},
  "entities": {
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
    app.add_plugins((Esp32Plugin, HealthPlugin, LedPlugin, WifiPlugin::from_env()));
    app.init_resource::<AppTypeRegistry>();
    app.register_type::<HelloWorld>();
    app.spawn((HttpServer::new(8080), Handler));
    app.add_systems(
        Startup,
        (setup_led, ping, dump_canonical, load_scene, greet).chain(),
    );
    app.run();
}

fn setup_led(mut commands: Commands) {
    commands.spawn((LedColor::default(), HueFade::default(), Ws2812Led));
}

fn ping(world: &mut World) {
    let spawner = *world.non_send::<Spawner>();
    async_bridge::spawn_driver(spawner, async move {
        loop {
            match Request::get("http://example.com").send().await {
                Ok(response) => {
                    info!("client GET example.com -> status {}", response.status().as_u16())
                }
                Err(e) => warn!("client request failed: {}", Debug2Format(&e)),
            }
            Timer::after(Duration::from_secs(15)).await;
        }
    });
}

/// The server's request handler.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Handler(cx: In<ActionContext<Request>>) -> Response {
    info!("server request on `{}`", cx.input.path_string().as_str());
    Response::ok_text("hello from the beet_esp kitchen sink\n")
}

fn dump_canonical(world: &mut World) {
    let temp = world.spawn(HelloWorld).id();
    match WorldSerdeSaver::new(world)
        .with_entity_tree(temp)
        .save(MediaType::Json)
    {
        Ok(bytes) => match bytes.as_utf8() {
            Ok(text) => info!("canonical scene JSON:\n{}", text),
            Err(e) => warn!("serialized scene not UTF-8: {}", Debug2Format(&e)),
        },
        Err(e) => warn!("failed to serialize scene: {}", Debug2Format(&e)),
    }
    world.despawn(temp);
}

fn load_scene(world: &mut World) {
    let bytes = MediaBytes::new_json(SCENE_JSON);
    match WorldSerdeLoader::new(world).load(&bytes) {
        Ok(spawned) => info!("loaded {} entit(ies) from JSON scene", spawned.len()),
        Err(e) => warn!("failed to load scene: {}", Debug2Format(&e)),
    }
}

fn greet(query: Query<Entity, With<HelloWorld>>) {
    info!("{} entit(ies) with HelloWorld present", query.iter().count());
}
