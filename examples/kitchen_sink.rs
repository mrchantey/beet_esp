//! Everything at once: LED + Wi-Fi (client & server) + `world_serde`, under the
//! health monitor. Exists to answer "how close to the hardware ceiling are we if
//! we combine all the features?" — flash it and read the periodic health report.
//!
//! It deliberately stacks the heaviest things we do: the Wi-Fi/embassy-net stack,
//! the on-board LED RMT driver, and reflection-backed scene (de)serialization,
//! all on one Bevy `World`.
//!
//! Run with:
//!   `cargo run --release --no-default-features --features led,wifi,action --example kitchen_sink`
//!
//! The default 96 KiB heap is *not* enough for this combination (Wi-Fi alone is
//! ~165 KiB of the 172 KiB total, and the action layer + task pool add more), so
//! the heap is bumped below — see the health report for the resulting headroom.

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

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

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

// The action layer + task pool + Wi-Fi push the baseline up, but the bulk now
// lives in PSRAM (see `beet_esp::mem`), so internal SRAM only holds the radio +
// DMA + stack. The default 64 KiB internal reserve (+ the ~72 KiB reclaimed
// region) is ample; the stack only ever uses ~19 KiB while serving.
#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    let mut app = App::new();
    app.add_plugins((
        Esp32Plugin,
        HealthPlugin,
        LedPlugin,
        WifiPlugin::new(SSID, PASSWORD),
    ));
    app.init_resource::<AppTypeRegistry>();
    app.register_type::<HelloWorld>();
    // beet's standard server bundle: the `HttpServer` component plus its
    // `Action<Request, Response>` handler. Spawning it starts the accept loop.
    app.spawn((HttpServer::new(8080), Handler));
    app.add_systems(
        Startup,
        (setup_led, ping, dump_canonical, load_scene, greet).chain(),
    );
    app.run();
}

fn setup_led(mut commands: Commands) {
    commands.spawn((LedColor::default(), HueFade::default()));
}

fn ping(world: &mut World) {
    let spawner = *world.non_send::<Spawner>();
    spawn_driver(spawner, async move {
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

/// The server's request handler: a beet `Action<Request, Response>` on the
/// server entity, dispatched by `entity.exchange`.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Handler(cx: In<ActionContext<Request>>) -> Response {
    info!("server request on `{}`", cx.input.path_string().as_str());
    Response::ok_body("hello from the beet_esp kitchen sink\n", MediaType::Text)
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
