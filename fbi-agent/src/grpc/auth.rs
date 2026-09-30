//! Caller authentication for the gRPC services.
//!
//! Production, staging and every preview slot bind their agent to loopback and
//! run as separate users, so reaching the port proves nothing: without a check,
//! staging or preview code could force-shut the production agent or make it
//! play clips. Every caller (web-server's Jammer calls, the deploy engine's
//! Admin calls) therefore presents its environment's `FBI_AGENT_REGISTRY_SECRET`
//! under [`INTERNAL_SECRET_HEADER`].

use std::sync::Arc;

use sakiot_proto::INTERNAL_SECRET_HEADER;
use subtle::ConstantTimeEq;
use tonic::{Request, Status, service::Interceptor};
use tracing::warn;

use super::GrpcServerError;

#[derive(Clone)]
pub(super) struct RequireSecret {
    expected: Option<Arc<str>>,
}

impl RequireSecret {
    /// Deployed agents are release builds and refuse to serve without a
    /// secret. A debug build may run open for local development.
    pub(super) fn resolve(
        configured: Option<String>,
        allow_open: bool,
    ) -> Result<Self, GrpcServerError> {
        match configured {
            Some(secret) => Ok(Self {
                expected: Some(secret.into()),
            }),
            None if allow_open => {
                warn!(
                    "FBI_AGENT_REGISTRY_SECRET is unset; the gRPC server accepts unauthenticated calls"
                );
                Ok(Self { expected: None })
            }
            None => Err(GrpcServerError::MissingSecret),
        }
    }
}

impl Interceptor for RequireSecret {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(expected) = &self.expected else {
            return Ok(request);
        };
        let presented = request
            .metadata()
            .get(INTERNAL_SECRET_HEADER)
            .map(|value| value.as_bytes());
        if presented.is_some_and(|actual| bool::from(actual.ct_eq(expected.as_bytes()))) {
            Ok(request)
        } else {
            warn!("rejected a gRPC call without a valid internal secret");
            Err(Status::unauthenticated(
                "missing or invalid internal secret",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::proto::{
        DrainRequest, DrainStatus, Empty,
        admin_client::AdminClient,
        admin_server::{Admin, AdminServer},
    };
    use tonic::{Code, Response, transport::Server, transport::server::TcpIncoming};

    fn with_secret(secret: Option<&str>) -> Request<()> {
        let mut request = Request::new(());
        if let Some(secret) = secret {
            request
                .metadata_mut()
                .insert(INTERNAL_SECRET_HEADER, secret.parse().unwrap());
        }
        request
    }

    fn requiring(secret: &str) -> RequireSecret {
        RequireSecret::resolve(Some(secret.into()), false).unwrap()
    }

    #[test]
    fn a_configured_secret_admits_only_callers_presenting_it() {
        let mut auth = requiring("s3cret");
        assert!(auth.call(with_secret(Some("s3cret"))).is_ok());

        for presented in [None, Some("wrong"), Some("s3cret-but-longer"), Some("")] {
            let status = auth.call(with_secret(presented)).unwrap_err();
            assert_eq!(status.code(), Code::Unauthenticated, "{presented:?}");
        }
    }

    #[test]
    fn a_missing_secret_is_fatal_unless_open_access_is_allowed() {
        assert!(matches!(
            RequireSecret::resolve(None, false),
            Err(GrpcServerError::MissingSecret)
        ));

        let mut open = RequireSecret::resolve(None, true).unwrap();
        assert!(open.call(with_secret(None)).is_ok());
    }

    struct StubAdmin;

    #[tonic::async_trait]
    impl Admin for StubAdmin {
        async fn start_drain(
            &self,
            _: Request<DrainRequest>,
        ) -> Result<Response<DrainStatus>, Status> {
            Ok(Response::new(DrainStatus::default()))
        }
        async fn cancel_drain(
            &self,
            _: Request<DrainRequest>,
        ) -> Result<Response<DrainStatus>, Status> {
            Ok(Response::new(DrainStatus::default()))
        }
        async fn get_drain_status(
            &self,
            _: Request<Empty>,
        ) -> Result<Response<DrainStatus>, Status> {
            Ok(Response::new(DrainStatus::default()))
        }
        async fn shutdown_when_empty(
            &self,
            _: Request<DrainRequest>,
        ) -> Result<Response<DrainStatus>, Status> {
            Ok(Response::new(DrainStatus::default()))
        }
        async fn force_shutdown(
            &self,
            _: Request<DrainRequest>,
        ) -> Result<Response<DrainStatus>, Status> {
            Ok(Response::new(DrainStatus::default()))
        }
    }

    #[tokio::test]
    async fn the_secret_travels_as_grpc_metadata() {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = incoming.local_addr().unwrap();
        let server = tokio::spawn(
            Server::builder()
                .add_service(AdminServer::with_interceptor(
                    StubAdmin,
                    requiring("s3cret"),
                ))
                .serve_with_incoming(incoming),
        );
        let mut client = AdminClient::connect(format!("http://{address}"))
            .await
            .unwrap();

        let denied = client.force_shutdown(DrainRequest::default()).await;
        assert_eq!(denied.unwrap_err().code(), Code::Unauthenticated);

        let mut request = Request::new(DrainRequest::default());
        request
            .metadata_mut()
            .insert(INTERNAL_SECRET_HEADER, "s3cret".parse().unwrap());
        assert!(client.force_shutdown(request).await.is_ok());

        server.abort();
    }
}
