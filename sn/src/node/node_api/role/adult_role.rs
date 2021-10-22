// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::messaging::{
    system::{NodeCmd, SystemMsg},
    MessageId,
};
use crate::node::{
    network::Network as NetworkApi,
    node_ops::{NodeDuties, NodeDuty},
    Result,
};
use crate::routing::{XorName, CHUNK_COPY_COUNT};
use crate::types::{Chunk, ChunkAddress};
use itertools::Itertools;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{info, trace, warn};

#[derive(Clone)]
pub(crate) struct AdultRole {
    pub(crate) network_api: NetworkApi,
}

impl AdultRole {
    #[allow(clippy::mutable_key_type)]
    pub(crate) async fn reorganize_chunks(
        &self,
        our_name: XorName,
        new_adults: BTreeSet<XorName>,
        lost_adults: BTreeSet<XorName>,
        remaining: BTreeSet<XorName>,
    ) -> Result<NodeDuties> {
        let chunks = self.network_api.get_chunk_storage().await;
        let keys = chunks.keys()?;
        let mut data_for_replication = BTreeMap::new();
        for addr in keys.iter() {
            if let Some((data, holders)) = self
                .republish_and_cache(addr, &our_name, &new_adults, &lost_adults, &remaining)
                .await
            {
                let _prev = data_for_replication.insert(data, holders);
            }
        }
        Ok(data_for_replication
            .into_iter()
            .map(|(data, targets)| NodeDuty::SendToNodes {
                msg_id: MessageId::new(),
                msg: SystemMsg::NodeCmd(NodeCmd::ReplicateChunk(data)),
                targets,
                aggregation: false,
            })
            .collect::<Vec<_>>())
    }

    async fn republish_and_cache(
        &self,
        address: &ChunkAddress,
        our_name: &XorName,
        new_adults: &BTreeSet<XorName>,
        lost_adults: &BTreeSet<XorName>,
        remaining: &BTreeSet<XorName>,
    ) -> Option<(Chunk, BTreeSet<XorName>)> {
        let chunks = self.network_api.get_chunk_storage().await;

        let old_adult_list = remaining.union(lost_adults).copied().collect();
        let new_adult_list = remaining.union(new_adults).copied().collect();
        let new_holders = self.compute_holders(address, &new_adult_list);
        let old_holders = self.compute_holders(address, &old_adult_list);

        let we_are_not_holder_anymore = !new_holders.contains(our_name);
        let new_adult_is_holder = !new_holders.is_disjoint(new_adults);
        let lost_old_holder = !old_holders.is_disjoint(lost_adults);

        if we_are_not_holder_anymore || new_adult_is_holder || lost_old_holder {
            info!("Republishing chunk at {:?}", address);
            trace!("We are not a holder anymore? {}, New Adult is Holder? {}, Lost Adult was holder? {}", we_are_not_holder_anymore, new_adult_is_holder, lost_old_holder);
            let chunk = chunks.get_chunk(address).ok()?;
            if we_are_not_holder_anymore {
                if let Err(err) = chunks.remove_chunk(address) {
                    warn!("Error deleting chunk during republish: {:?}", err);
                }
            }
            // TODO: Push to LRU cache
            Some((chunk, new_holders))
        } else {
            None
        }
    }

    fn compute_holders(
        &self,
        addr: &ChunkAddress,
        adult_list: &BTreeSet<XorName>,
    ) -> BTreeSet<XorName> {
        adult_list
            .iter()
            .sorted_by(|lhs, rhs| addr.name().cmp_distance(lhs, rhs))
            .take(CHUNK_COPY_COUNT)
            .cloned()
            .collect()
    }
}