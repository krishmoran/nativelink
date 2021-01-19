// Copyright 2020-2021 Nathan (Blaise) Bruer.  All rights reserved.

use std::collections::HashMap;
use std::convert::TryInto;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use prost::Message;
use tonic::{Request, Response, Status};

use proto::build::bazel::remote::execution::v2::{
    action_cache_server::ActionCache, action_cache_server::ActionCacheServer as Server, ActionResult,
    GetActionResultRequest, UpdateActionResultRequest,
};

use common::{log, DigestInfo};
use config::cas_server::{AcStoreConfig, InstanceName};
use error::{make_input_err, Code, Error, ResultExt};
use store::{Store, StoreManager};

pub struct AcServer {
    ac_store: Arc<dyn Store>,
}

impl AcServer {
    pub fn new(config: &HashMap<InstanceName, AcStoreConfig>, store_manager: &StoreManager) -> Result<Self, Error> {
        for (_instance_name, ac_cfg) in config {
            let ac_store = store_manager
                .get_store(&ac_cfg.ac_store)
                .ok_or_else(|| make_input_err!("'ac_store': '{}' does not exist", ac_cfg.ac_store))?;
            return Ok(AcServer {
                ac_store: ac_store.clone(),
            });
        }
        Err(make_input_err!("No configuration configured for 'ac' service"))
    }

    pub fn into_service(self) -> Server<AcServer> {
        Server::new(self)
    }

    async fn inner_get_action_result(
        &self,
        grpc_request: Request<GetActionResultRequest>,
    ) -> Result<Response<ActionResult>, Error> {
        let get_action_request = grpc_request.into_inner();

        // TODO(blaise.bruer) This needs to be fixed. It is using wrong macro.
        // We also should write a test for these errors.
        let digest: DigestInfo = get_action_request
            .action_digest
            .err_tip(|| "Action digest was not set in message")?
            .try_into()?;

        // TODO(allada) There is a security risk here of someone taking all the memory on the instance.
        let mut store_data = Vec::with_capacity(digest.size_bytes as usize);
        let mut cursor = Cursor::new(&mut store_data);
        let ac_store = Pin::new(self.ac_store.as_ref());
        ac_store.get(digest.clone(), &mut cursor).await?;

        let action_result = ActionResult::decode(Cursor::new(&store_data))
            .err_tip_with_code(|e| (Code::NotFound, format!("Stored value appears to be corrupt: {}", e)))?;

        Ok(Response::new(action_result))
    }

    async fn inner_update_action_result(
        &self,
        grpc_request: Request<UpdateActionResultRequest>,
    ) -> Result<Response<ActionResult>, Error> {
        let update_action_request = grpc_request.into_inner();

        // TODO(blaise.bruer) This needs to be fixed. It is using wrong macro.
        // We also should write a test for these errors.
        let digest: DigestInfo = update_action_request
            .action_digest
            .err_tip(|| "Action digest was not set in message")?
            .try_into()?;

        let action_result = update_action_request
            .action_result
            .err_tip(|| "Action result was not set in message")?;

        // TODO(allada) There is a security risk here of someone taking all the memory on the instance.
        let mut store_data = Vec::new();
        action_result
            .encode(&mut store_data)
            .err_tip(|| "Provided ActionResult could not be serialized")?;

        let ac_store = Pin::new(self.ac_store.as_ref());
        ac_store.update(digest, Box::new(Cursor::new(store_data))).await?;
        Ok(Response::new(action_result))
    }
}

#[tonic::async_trait]
impl ActionCache for AcServer {
    async fn get_action_result(
        &self,
        grpc_request: Request<GetActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        let now = Instant::now();
        log::info!("\x1b[0;31mget_action_result Req\x1b[0m: {:?}", grpc_request.get_ref());
        let resp = self.inner_get_action_result(grpc_request).await;
        let d = now.elapsed().as_secs_f32();
        log::info!("\x1b[0;31mget_action_result Resp\x1b[0m: {} {:?}", d, resp);
        return resp.map_err(|e| e.into());
    }

    async fn update_action_result(
        &self,
        grpc_request: Request<UpdateActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        let now = Instant::now();
        log::info!(
            "\x1b[0;31mupdate_action_result Req\x1b[0m: {:?}",
            grpc_request.get_ref()
        );
        let resp = self.inner_update_action_result(grpc_request).await;
        let d = now.elapsed().as_secs_f32();
        log::info!("\x1b[0;31mupdate_action_result Resp\x1b[0m: {} {:?}", d, resp);
        return resp.map_err(|e| e.into());
    }
}
