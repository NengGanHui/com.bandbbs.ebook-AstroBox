use crate::exports::astrobox::psys_plugin::{
    event::{self, EventType},
    lifecycle,
};
use wit_bindgen::FutureReader;

pub mod chapters;
pub mod logger;
pub mod protocol;
pub mod ui;
wit_bindgen::generate!({
    path: "wit",
    world: "psys-world",
    generate_all,
});

struct MyPlugin;

impl event::Guest for MyPlugin {
    #[allow(async_fn_in_trait)]
    fn on_event(event_type: EventType, event_payload: _rt::String) -> FutureReader<String> {
        let (writer, reader) = wit_future::new::<String>(|| "".to_string());

        match event_type {
            EventType::InterconnectMessage => {
                ui::handle_interconnect_message(&event_payload);
            }
            EventType::Timer => {
                ui::handle_timer_event(&event_payload);
            }
            EventType::PluginMessage
            | EventType::DeviceAction
            | EventType::ProviderAction
            | EventType::DeeplinkAction
            | EventType::TransportPacket => {}
        }

        tracing::info!("event_payload: {}", event_payload);

        wit_bindgen::spawn(async move {
            let _ = writer.write("".to_string()).await;
        });

        reader
    }

    fn on_ui_event(
        event_id: _rt::String,
        event: event::Event,
        event_payload: _rt::String,
    ) -> wit_bindgen::rt::async_support::FutureReader<_rt::String> {
        let (writer, reader) = wit_future::new::<String>(|| "".to_string());

        ui::ui_event_processor(event, &event_id, &event_payload);

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
        tracing::info!("弦电子书同步器 v2 已加载（目标：弦电子书 plus）");
    }
}

export!(MyPlugin);
