//! Adapters between component-model `stream<T>` and Rust streams/channels.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::BoxStream;
use tokio::sync::mpsc;
use wasmtime::StoreContextMut;
use wasmtime::component::{
    Destination, Lift, Source, StreamConsumer, StreamProducer, StreamResult, VecBuffer,
};

/// Host-side writer of a guest-read `stream<T>` fed by a Rust stream of items.
pub(crate) struct ItemProducer<T: Send + 'static> {
    inner: BoxStream<'static, T>,
}

impl<T: Send + 'static> ItemProducer<T> {
    pub(crate) fn new(inner: BoxStream<'static, T>) -> Self {
        Self { inner }
    }
}

impl<T: Send + Sync + 'static, D: 'static> StreamProducer<D> for ItemProducer<T> {
    type Item = T;
    type Buffer = VecBuffer<T>;

    fn poll_produce<'a>(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        store: StoreContextMut<'a, D>,
        mut dst: Destination<'a, T, VecBuffer<T>>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let capacity = dst.remaining(store).unwrap_or(1).max(1);
        let mut items = Vec::new();
        while items.len() < capacity {
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => items.push(item),
                Poll::Ready(None) => {
                    if !items.is_empty() {
                        dst.set_buffer(items.into());
                    }
                    return Poll::Ready(Ok(StreamResult::Dropped));
                }
                Poll::Pending => break,
            }
        }
        if items.is_empty() {
            return if finish {
                Poll::Ready(Ok(StreamResult::Cancelled))
            } else {
                Poll::Pending
            };
        }
        dst.set_buffer(items.into());
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

/// Host-side writer of a guest-read `stream<u8>` fed by byte chunks.
pub(crate) struct ByteProducer {
    inner: BoxStream<'static, Vec<u8>>,
}

impl ByteProducer {
    pub(crate) fn new(inner: BoxStream<'static, Vec<u8>>) -> Self {
        Self { inner }
    }
}

impl<D: 'static> StreamProducer<D> for ByteProducer {
    type Item = u8;
    type Buffer = VecBuffer<u8>;

    fn poll_produce<'a>(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: StoreContextMut<'a, D>,
        mut dst: Destination<'a, u8, VecBuffer<u8>>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        loop {
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(chunk)) if chunk.is_empty() => {}
                Poll::Ready(Some(chunk)) => {
                    dst.set_buffer(chunk.into());
                    return Poll::Ready(Ok(StreamResult::Completed));
                }
                Poll::Ready(None) => return Poll::Ready(Ok(StreamResult::Dropped)),
                Poll::Pending => {
                    return if finish {
                        Poll::Ready(Ok(StreamResult::Cancelled))
                    } else {
                        Poll::Pending
                    };
                }
            }
        }
    }
}

/// Host-side reader of a guest-written `stream<T>`, forwarding items into a channel.
/// Dropping the receiving end stops the stream (`Dropped`), which cancels the guest writer.
pub(crate) struct ChannelConsumer<T> {
    tx: mpsc::UnboundedSender<T>,
}

impl<T> ChannelConsumer<T> {
    pub(crate) fn new(tx: mpsc::UnboundedSender<T>) -> Self {
        Self { tx }
    }
}

impl<T: Lift + Send + Sync + 'static, D: 'static> StreamConsumer<D> for ChannelConsumer<T> {
    type Item = T;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut store: StoreContextMut<D>,
        mut source: Source<'_, T>,
        _finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        if self.tx.is_closed() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        let available = source.remaining(&mut store);
        let mut items: Vec<T> = Vec::with_capacity(available.max(1));
        source.read(&mut store, &mut items)?;
        for item in items {
            if self.tx.send(item).is_err() {
                return Poll::Ready(Ok(StreamResult::Dropped));
            }
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

/// Converts an unbounded receiver into a boxed stream.
pub(crate) fn receiver_stream<T: Send + 'static>(
    mut rx: mpsc::UnboundedReceiver<T>,
) -> BoxStream<'static, T> {
    Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)))
}
