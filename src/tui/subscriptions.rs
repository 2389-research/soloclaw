// ABOUTME: Custom boba SubscriptionSource for agent loop events.
// ABOUTME: Wraps the mpsc::Receiver<AgentEvent> so boba's runtime manages it.

use std::sync::Arc;

use boba::{SubscriptionId, SubscriptionSource};
use futures::stream::BoxStream;
use futures::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;

use crate::tui::state::AgentEvent;

/// Subscription source that bridges the agent loop's mpsc channel into boba's
/// subscription system. The receiver is wrapped in `Arc<Mutex<Option<...>>>`
/// because boba calls `subscriptions()` on every update cycle, but the stream
/// is only consumed once — when the subscription first starts via `stream()`.
/// On subsequent cycles boba sees the same `SubscriptionId` and keeps the
/// existing subscription alive without calling `stream()` again.
pub struct AgentEventSource {
    pub rx: Arc<Mutex<Option<mpsc::Receiver<AgentEvent>>>>,
}

impl SubscriptionSource for AgentEventSource {
    type Output = AgentEvent;

    fn id(&self) -> SubscriptionId {
        SubscriptionId::of::<Self>()
    }

    fn stream(self) -> BoxStream<'static, AgentEvent> {
        Box::pin(
            futures::stream::once(async move { self.rx.lock().await.take() })
                .filter_map(|opt| async { opt })
                .map(ReceiverStream::new)
                .flatten(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_id_is_stable_across_instances() {
        let (_tx1, rx1) = mpsc::channel::<AgentEvent>(1);
        let source1 = AgentEventSource {
            rx: Arc::new(Mutex::new(Some(rx1))),
        };

        let (_tx2, rx2) = mpsc::channel::<AgentEvent>(1);
        let source2 = AgentEventSource {
            rx: Arc::new(Mutex::new(Some(rx2))),
        };

        assert_eq!(source1.id(), source2.id());
    }

    #[tokio::test]
    async fn stream_delivers_events() {
        let (tx, rx) = mpsc::channel::<AgentEvent>(16);
        let source = AgentEventSource {
            rx: Arc::new(Mutex::new(Some(rx))),
        };

        let mut stream = source.stream();

        tx.send(AgentEvent::TextDelta("hello".to_string()))
            .await
            .unwrap();
        tx.send(AgentEvent::Done).await.unwrap();

        let first = stream.next().await.expect("expected first event");
        assert!(
            matches!(first, AgentEvent::TextDelta(ref s) if s == "hello"),
            "expected TextDelta(\"hello\"), got {:?}",
            std::mem::discriminant(&first),
        );

        let second = stream.next().await.expect("expected second event");
        assert!(
            matches!(second, AgentEvent::Done),
            "expected Done event",
        );
    }
}
