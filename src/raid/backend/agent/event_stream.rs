use std::sync::Arc;

use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::Stream;

use super::types::{AssistantMessage, AssistantMessageEvent};

struct EventStreamInner<T, R> {
    sender: tokio::sync::mpsc::UnboundedSender<T>,
    receiver: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<T>>>,
    done: std::sync::Mutex<bool>,
    result: std::sync::Mutex<Option<R>>,
    is_complete: Arc<dyn Fn(&T) -> bool + Send + Sync>,
    extract_result: Arc<dyn Fn(T) -> R + Send + Sync>,
    notify: Arc<tokio::sync::Notify>,
}

pub struct EventStream<T, R> {
    inner: Arc<EventStreamInner<T, R>>,
}

impl<T: Clone + Send + 'static, R: Send + 'static> EventStream<T, R> {
    pub fn new(
        is_complete: impl Fn(&T) -> bool + Send + Sync + 'static,
        extract_result: impl Fn(T) -> R + Send + Sync + 'static,
    ) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            inner: Arc::new(EventStreamInner {
                sender,
                receiver: std::sync::Mutex::new(Some(receiver)),
                done: std::sync::Mutex::new(false),
                result: std::sync::Mutex::new(None),
                is_complete: Arc::new(is_complete),
                extract_result: Arc::new(extract_result),
                notify: Arc::new(tokio::sync::Notify::new()),
            }),
        }
    }

    pub fn push(&self, event: T) {
        {
            let mut done = self.inner.done.lock().expect("event stream done lock");
            if *done {
                return;
            }
            if (self.inner.is_complete)(&event) {
                *done = true;
                *self.inner.result.lock().expect("event stream result lock") =
                    Some((self.inner.extract_result)(event.clone()));
            }
        }
        let _ = self.inner.sender.send(event);
        self.inner.notify.notify_waiters();
    }

    pub fn end(&self, result: Option<R>) {
        *self.inner.done.lock().expect("event stream done lock") = true;
        if let Some(result) = result {
            *self.inner.result.lock().expect("event stream result lock") = Some(result);
        }
        self.inner.notify.notify_waiters();
    }

    pub fn into_stream(self) -> impl Stream<Item = T> {
        let mut receiver_slot = self
            .inner
            .receiver
            .lock()
            .expect("event stream receiver lock");
        let receiver = Option::take(&mut *receiver_slot).expect("event stream receiver already taken");
        UnboundedReceiverStream::new(receiver)
    }
}

impl<T, R> Clone for EventStream<T, R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub type AssistantMessageStream = EventStream<AssistantMessageEvent, AssistantMessage>;

pub fn assistant_message_stream() -> AssistantMessageStream {
    EventStream::new(
        |event| matches!(event, AssistantMessageEvent::Done { .. } | AssistantMessageEvent::Error { .. }),
        |event| match event {
            AssistantMessageEvent::Done { message, .. } => message,
            AssistantMessageEvent::Error { error, .. } => error,
            _ => unreachable!("completion predicate guarantees done or error"),
        },
    )
}
