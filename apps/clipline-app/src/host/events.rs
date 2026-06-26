use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ClientEvent {
    pub name: &'static str,
    pub payload: serde_json::Value,
}

impl ClientEvent {
    pub fn new(name: &'static str, payload: serde_json::Value) -> Self {
        Self { name, payload }
    }
}

#[derive(Default)]
pub struct ClientEventHub {
    subscribers: Mutex<Vec<ClientEventSubscriber>>,
    next_subscriber_id: AtomicU64,
}

struct ClientEventSubscriber {
    id: u64,
    sender: Sender<ClientEvent>,
}

impl ClientEventHub {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn subscribe(&self) -> Receiver<ClientEvent> {
        self.subscribe_with_id().1
    }

    pub fn subscribe_with_id(&self) -> (u64, Receiver<ClientEvent>) {
        let (tx, rx) = mpsc::channel();
        let id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(ClientEventSubscriber { id, sender: tx });
        }
        (id, rx)
    }

    pub fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.id != id);
        }
    }

    pub fn emit(&self, event: ClientEvent) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.sender.send(event.clone()).is_ok());
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map(|subscribers| subscribers.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_hub_fans_out_to_multiple_subscribers() {
        let hub = ClientEventHub::default();
        let first = hub.subscribe();
        let second = hub.subscribe();

        hub.emit(ClientEvent::new(
            "status",
            serde_json::json!({"recording": true}),
        ));

        assert_eq!(first.try_recv().unwrap().name, "status");
        assert_eq!(second.try_recv().unwrap().name, "status");
    }

    #[test]
    fn event_hub_drops_disconnected_subscribers() {
        let hub = ClientEventHub::default();
        let dropped = hub.subscribe();
        drop(dropped);
        let live = hub.subscribe();

        hub.emit(ClientEvent::new(
            "saved",
            serde_json::json!({"path": "clip.mp4"}),
        ));

        assert_eq!(live.try_recv().unwrap().name, "saved");
        assert_eq!(hub.subscriber_count(), 1);
    }
}
