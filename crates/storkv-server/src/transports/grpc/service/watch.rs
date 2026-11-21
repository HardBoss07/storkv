//! Watch-stream support for the `Storkv` gRPC service.

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};

use super::super::conversions::convert_event;
use super::super::{
    ChangeEvent, GrpcService, WATCH_CHANNEL_CAPACITY, WATCH_POLL_INTERVAL, WatchRequest,
};

pub(super) type WatchStream =
    std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<ChangeEvent, Status>> + Send>>;

pub(super) fn watch_stream(
    service: &GrpcService,
    request: GrpcRequest<WatchRequest>,
) -> Result<GrpcResponse<WatchStream>, Status> {
    let (scope_id, all_scopes, from_seq) = {
        let message = request.get_ref();
        (message.scope_id, message.all_scopes, message.from_seq)
    };
    let receiver = if from_seq > 0 {
        service.server.db().watch_from(from_seq).map_err(|error| {
            super::super::status_from_wire(crate::dispatch::core_error_to_wire(error))
        })?
    } else {
        service.server.db().watch().map_err(|error| {
            super::super::status_from_wire(crate::dispatch::core_error_to_wire(error))
        })?
    };
    let (sender, stream_receiver) =
        mpsc::channel::<Result<ChangeEvent, Status>>(WATCH_CHANNEL_CAPACITY);
    let _watch_task = tokio::task::spawn_blocking(move || {
        while !sender.is_closed() {
            match receiver.recv_timeout(WATCH_POLL_INTERVAL) {
                Ok(event) => {
                    let Some(event) = convert_event(event) else {
                        continue;
                    };
                    if !all_scopes && event.scope_id != scope_id {
                        continue;
                    }
                    if sender.try_send(Ok(event)).is_err() {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let stream = ReceiverStream::new(stream_receiver);
    Ok(GrpcResponse::new(Box::pin(stream)))
}
