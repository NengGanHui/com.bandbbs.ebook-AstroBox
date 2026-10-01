use crate::exports::astrobox::psys_plugin::{event_v3 as event, lifecycle};
use wit_bindgen::FutureReader;

pub mod chapters;
pub mod logger;
pub mod protocol;
pub mod ui;
wit_bindgen::generate!({
    path: "wit",
    world: "psys-world-v3",
    generate_all,
});

struct MyPlugin;

impl event::Guest for MyPlugin {
    #[allow(async_fn_in_trait)]
    fn on_event(
        event_type: event::EventType,
        event_payload: _rt::String,
    ) -> FutureReader<String> {
        let (writer, reader) = wit_future::new::<String>(|| "".to_string());

        match event_type {
            event::EventType::InterconnectMessage => {
                ui::handle_interconnect_message(&event_payload);
            }
            event::EventType::Timer => {
                ui::handle_timer_event(&event_payload);
            }
            event::EventType::PluginMessage
            | event::EventType::DeviceAction
            | event::EventType::ProviderAction
            | event::EventType::DeeplinkAction
            | event::EventType::TransportPacket => {}
        }

        tracing::info!("event_payload: {}", event_payload);

        wit_bindgen::spawn(async move {
            let _ = writer.write("".to_string()).await;
        });

        reader
    }

    fn on_ui_event_v3(
        event_id: _rt::String,
        event_type: event::Event,
        event_payload: _rt::String,
    ) -> wit_bindgen::rt::async_support::FutureReader<_rt::String> {
        let (writer, reader) = wit_future::new::<String>(|| "".to_string());

        ui::ui_event_processor(event_type, &event_id, &event_payload);

        wit_bindgen::spawn(async move {
            let _ = writer.write("".to_string()).await;
        });

        reader
    }

    fn on_ui_render(element_id: _rt::String) -> wit_bindgen::rt::async_support::FutureReader<()> {
        let (writer, reader) = wit_future::new::<()>(|| ());

        ui::render_main_ui(&element_id);

        wit_bindgen::spawn(async move {
            let _ = writer.write(()).await;
        });

        reader
    }

    fn on_card_render(_card_id: _rt::String) -> wit_bindgen::rt::async_support::FutureReader<()> {
        let (writer, reader) = wit_future::new::<()>(|| ());

        wit_bindgen::spawn(async move {
            let _ = writer.write(()).await;
        });

        reader
    }
}

impl lifecycle::Guest for MyPlugin {
    #[allow(async_fn_in_trait)]
    fn on_load() -> () {
        logger::init();
        tracing::info!("弦电子书同步器已加载");
    }
}

export!(MyPlugin);
