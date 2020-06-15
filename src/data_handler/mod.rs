// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod idata_handler;
mod idata_holder;
mod mdata_handler;
mod sdata_handler;

use crate::{action::Action, rpc::Rpc, utils, vault::Init, Config, Result};
use idata_handler::IDataHandler;
use idata_holder::IDataHolder;
use mdata_handler::MDataHandler;
use routing::{Node, SrcLocation};
use sdata_handler::SDataHandler;

use log::{debug, error, trace};
use safe_nd::{
    IDataAddress, IDataRequest, MessageId, NodePublicId, PublicId, Request, Response, XorName,
};
use threshold_crypto::{Signature, SignatureShare};

use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    fmt::{self, Display, Formatter},
    rc::Rc,
};

pub(crate) struct DataHandler {
    id: NodePublicId,
    idata_holder: IDataHolder,
    idata_handler: Option<IDataHandler>,
    mdata_handler: Option<MDataHandler>,
    sdata_handler: Option<SDataHandler>,
    routing_node: Rc<RefCell<Node>>,
}

impl DataHandler {
    pub fn new(
        id: NodePublicId,
        config: &Config,
        total_used_space: &Rc<Cell<u64>>,
        init_mode: Init,
        is_elder: bool,
        routing_node: Rc<RefCell<Node>>,
    ) -> Result<Self> {
        let idata_holder = IDataHolder::new(id.clone(), config, total_used_space, init_mode)?;
        let (idata_handler, mdata_handler, sdata_handler) = if is_elder {
            let idata_handler =
                IDataHandler::new(id.clone(), config, init_mode, routing_node.clone())?;
            let mdata_handler = MDataHandler::new(id.clone(), config, total_used_space, init_mode)?;
            let sdata_handler = SDataHandler::new(id.clone(), config, total_used_space, init_mode)?;

            (
                Some(idata_handler),
                Some(mdata_handler),
                Some(sdata_handler),
            )
        } else {
            (None, None, None)
        };

        Ok(Self {
            id,
            idata_holder,
            idata_handler,
            mdata_handler,
            sdata_handler,
            routing_node,
        })
    }

    pub fn handle_vault_rpc(
        &mut self,
        src: SrcLocation,
        rpc: Rpc,
        accumulated_signature: Option<Signature>,
    ) -> Option<Action> {
        match rpc {
            Rpc::Request {
                request,
                requester,
                message_id,
                ..
            } => self.handle_request(src, requester, request, message_id, accumulated_signature),
            Rpc::Response {
                response,
                requester,
                message_id,
                proof,
                ..
            } => self.handle_response(src, response, requester, message_id, proof),
            Rpc::Duplicate {
                address,
                holders,
                message_id,
                ..
            } => self.handle_duplicate_request(address, holders, message_id, accumulated_signature),
            Rpc::DuplicationComplete {
                response,
                message_id,
                proof: Some((idata_address, signature)),
            } => self.complete_duplication(src, response, message_id, idata_address, signature),
            _ => None,
        }
    }

    fn complete_duplication(
        &mut self,
        sender: SrcLocation,
        response: Response,
        message_id: MessageId,
        idata_address: IDataAddress,
        signature: Signature,
    ) -> Option<Action> {
        use Response::*;
        if self
            .routing_node
            .borrow()
            .public_key_set()
            .ok()?
            .public_key()
            .verify(&signature, &utils::serialise(&idata_address))
        {
            match response {
                Mutation(result) => self.handle_idata_request(|idata_handler| {
                    idata_handler.update_idata_holders(
                        idata_address,
                        utils::get_source_name(sender),
                        result,
                        message_id,
                    )
                }),
                // Duplication doesn't care about other type of responses
                ref _other => {
                    error!(
                        "{}: Should not receive {:?} as a data handler.",
                        self, response
                    );
                    None
                }
            }
        } else {
            error!("Ignoring duplication response. Invalid Signature.");
            None
        }
    }

    fn handle_duplicate_request(
        &mut self,
        address: IDataAddress,
        holders: BTreeSet<XorName>,
        message_id: MessageId,
        accumulated_signature: Option<Signature>,
    ) -> Option<Action> {
        trace!(
            "Sending GetIData request for address: ({:?}) to {:?}",
            address,
            holders,
        );
        let our_id = self.id.clone();
        Some(Action::SendToPeers {
            targets: holders,
            rpc: Rpc::Request {
                request: Request::IData(IDataRequest::Get(address)),
                requester: PublicId::Node(our_id),
                message_id,
                signature: Some((0, SignatureShare(accumulated_signature?))),
            },
        })
    }

    fn validate_section_signature(&self, request: &Request, signature: &Signature) -> Option<()> {
        if self
            .routing_node
            .borrow()
            .public_key_set()
            .ok()?
            .public_key()
            .verify(signature, &utils::serialise(request))
        {
            Some(())
        } else {
            None
        }
    }

    fn handle_request(
        &mut self,
        src: SrcLocation,
        requester: PublicId,
        request: Request,
        message_id: MessageId,
        accumulated_signature: Option<Signature>,
    ) -> Option<Action> {
        use Request::*;
        trace!(
            "{}: Received ({:?} {:?}) from src {:?} (client {:?})",
            self,
            request,
            message_id,
            src,
            requester
        );
        match request.clone() {
            IData(idata_req) => {
                match idata_req {
                    IDataRequest::Put(data) => {
                        if src.is_section() {
                            // Since the requester is a section, this message was sent by the data handlers to us
                            // as a single data handler, implying that we're a data holder chosen to store the
                            // chunk.
                            if let Some(()) = self.validate_section_signature(
                                &request,
                                accumulated_signature.as_ref()?,
                            ) {
                                self.idata_holder.store_idata(
                                    src,
                                    &data,
                                    requester,
                                    message_id,
                                    accumulated_signature,
                                    request,
                                )
                            } else {
                                error!("Accumulated signature for {:?} is invalid!", &message_id);
                                None
                            }
                        } else {
                            self.handle_idata_request(|idata_handler| {
                                idata_handler
                                    .handle_put_idata_req(requester, data, message_id, request)
                            })
                        }
                    }
                    IDataRequest::Get(address) => {
                        if src.is_section() {
                            // Since the requester is a node, this message was sent by the data handlers to us
                            // as a single data handler, implying that we're a data holder where the chunk is
                            // stored.
                            if let Some(()) = self.validate_section_signature(
                                &request,
                                accumulated_signature.as_ref()?,
                            ) {
                                self.idata_holder.get_idata(
                                    src,
                                    address,
                                    requester,
                                    message_id,
                                    request,
                                    accumulated_signature,
                                )
                            } else {
                                error!("Accumulated signature is invalid!");
                                None
                            }
                        } else if matches!(requester, PublicId::Node(_)) {
                            if self
                                .routing_node
                                .borrow()
                                .public_key_set()
                                .ok()?
                                .public_key()
                                .verify(accumulated_signature.as_ref()?, utils::serialise(&address))
                            {
                                self.idata_holder.get_idata(
                                    src,
                                    address,
                                    requester,
                                    message_id,
                                    request,
                                    accumulated_signature,
                                )
                            } else {
                                error!("Accumulated signature is invalid!");
                                None
                            }
                        } else {
                            self.handle_idata_request(|idata_handler| {
                                idata_handler
                                    .handle_get_idata_req(requester, address, message_id, request)
                            })
                        }
                    }
                    IDataRequest::DeleteUnpub(address) => {
                        if src.is_section() {
                            // Since the requester is a node, this message was sent by the data handlers to us
                            // as a single data handler, implying that we're a data holder where the chunk is
                            // stored.
                            if let Some(()) = self.validate_section_signature(
                                &request,
                                accumulated_signature.as_ref()?,
                            ) {
                                self.idata_holder.delete_unpub_idata(
                                    address,
                                    requester,
                                    message_id,
                                    request,
                                    accumulated_signature,
                                )
                            } else {
                                error!("Accumulated signature is invalid!");
                                None
                            }
                        } else {
                            // We're acting as data handler, received request from client handlers
                            self.handle_idata_request(|idata_handler| {
                                idata_handler.handle_delete_unpub_idata_req(
                                    requester, address, message_id, request,
                                )
                            })
                        }
                    }
                }
            }
            MData(mdata_req) => self.mdata_handler.as_mut().map_or_else(
                || {
                    trace!("Not applicable to Adults");
                    None
                },
                |mdata_handler| mdata_handler.handle_request(requester, mdata_req, message_id),
            ),
            SData(sdata_req) => self.sdata_handler.as_mut().map_or_else(
                || {
                    trace!("Not applicable to Adults");
                    None
                },
                |sdata_handler| sdata_handler.handle_request(requester, sdata_req, message_id),
            ),
            Money(_) | LoginPacket(_) | Client(_) => {
                error!(
                    "{}: Should not receive {:?} as a data handler.",
                    self, request
                );
                None
            }
        }
    }

    fn handle_idata_request<F>(&mut self, operation: F) -> Option<Action>
    where
        F: FnOnce(&mut IDataHandler) -> Option<Action>,
    {
        self.idata_handler.as_mut().map_or_else(
            || {
                trace!("Not applicable to Adults");
                None
            },
            |idata_handler| operation(idata_handler),
        )
    }

    fn handle_response(
        &mut self,
        src: SrcLocation,
        response: Response,
        requester: PublicId,
        message_id: MessageId,
        proof: Option<(Request, Signature)>,
    ) -> Option<Action> {
        use Response::*;
        trace!(
            "{}: Received ({:?} {:?}) from {}",
            self,
            response,
            message_id,
            utils::get_source_name(src),
        );
        if let Some((request, signature)) = proof {
            if !matches!(requester, PublicId::Node(_))
                && self
                    .validate_section_signature(&request, &signature)
                    .is_none()
            {
                error!("Invalid section signature");
                return None;
            }
            match response {
                Mutation(result) => self.handle_idata_request(move |idata_handler| {
                    idata_handler.handle_mutation_resp(
                        utils::get_source_name(src),
                        requester,
                        result,
                        message_id,
                        request,
                    )
                }),
                GetIData(result) => {
                    if matches!(requester, PublicId::Node(_)) {
                        debug!("got the duplication copy");
                        if let Ok(data) = result {
                            trace!(
                                "Got GetIData copy response for address: ({:?})",
                                data.address(),
                            );
                            self.idata_holder.store_idata(
                                src,
                                &data,
                                requester,
                                message_id,
                                Some(signature),
                                request,
                            )
                        } else {
                            None
                        }
                    } else {
                        self.handle_idata_request(|idata_handler| {
                            idata_handler.handle_get_idata_resp(
                                result,
                                message_id,
                                requester,
                                (request, signature),
                            )
                        })
                    }
                }
                //
                // ===== Invalid =====
                //
                ref _other => {
                    error!(
                        "{}: Should not receive {:?} as a data handler.",
                        self, response
                    );
                    None
                }
            }
        } else {
            error!("Missing section signature");
            None
        }
    }

    // This should be called whenever a node leaves the section. It fetches the list of data that was
    // previously held by the node and requests the other holders to store an additional copy.
    // The list of holders is also updated by removing the node that left.
    pub fn trigger_chunk_duplication(&mut self, node: XorName) -> Option<Vec<Action>> {
        self.idata_handler.as_mut().map_or_else(
            || {
                trace!("Not applicable to Adults");
                None
            },
            |idata_handler| idata_handler.trigger_data_copy_process(node),
        )
    }
}

impl Display for DataHandler {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "{}", self.id.name())
    }
}
