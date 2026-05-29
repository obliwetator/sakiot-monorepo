use crate::Custom;

pub mod proto {
    tonic::include_proto!("fbi_agent");
}

mod admin;
mod dashboard;
mod jammer;
mod snapshot;

#[derive(Clone)]
pub struct FbiAgentGrpc {
    data_cache: Custom,
}

impl FbiAgentGrpc {
    pub fn new(data_cache: Custom) -> Self {
        Self { data_cache }
    }
}
