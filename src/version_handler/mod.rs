// Copyright 2015 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

#![allow(dead_code)]
use routing;
use maidsafe_types;
use routing::NameType;
use routing::error::{ResponseError, InterfaceError};
use routing::types::{Action, GROUP_SIZE};
use chunk_store::ChunkStore;
use routing::sendable::Sendable;
use rustc_serialize::{Decodable, Decoder, Encodable, Encoder};
use cbor;

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Debug)]
pub struct VersionHandlerSendable {
    name: NameType,
    tag: u64,
    data: Vec<u8>,
}

impl VersionHandlerSendable {
    pub fn new(name: NameType, data: Vec<u8>) -> VersionHandlerSendable {
        VersionHandlerSendable {
            name: name,
            tag: 209, // FIXME : Change once the tag is freezed
            data: data,
        }
    }

    pub fn get_data(&self) -> &Vec<u8> {
        &self.data
    }
}
impl Sendable for VersionHandlerSendable {
    fn name(&self) -> NameType {
        self.name.clone()
    }

    fn type_tag(&self) -> u64 {
        self.tag.clone()
    }

    fn serialised_contents(&self) -> Vec<u8> {
        let mut e = cbor::Encoder::from_memory();
        e.encode(&[&self]).unwrap();
        e.into_bytes()
    }

    fn refresh(&self) -> bool {
        true
    }

    fn merge(&self, responses: Vec<Box<Sendable>>) -> Option<Box<Sendable>> {
        let mut tmp_wrapper: VersionHandlerSendable;
        let mut data: Vec<u64> = Vec::new();
        for value in responses {
            let mut d = cbor::Decoder::from_bytes(value.serialised_contents());
            tmp_wrapper = d.decode().next().unwrap().unwrap();
            for val in tmp_wrapper.get_data().iter() {
                data.push(*val as u64);
            }
        }
        assert!(data.len() < (GROUP_SIZE as usize + 1) / 2);
        Some(Box::new(VersionHandlerSendable::new(NameType([0u8;64]),
            vec![super::utils::median(&data) as u8])))
    }

}

pub struct VersionHandler {
  // This is assuming ChunkStore has the ability of handling mutable(SDV) data, and put is overwritable
  // If such assumption becomes in-valid, LruCache or Sqlite based persona specific database shall be used
  chunk_store_ : ChunkStore
}

impl VersionHandler {
  pub fn new() -> VersionHandler {
    // TODO adjustable max_disk_space
    VersionHandler { chunk_store_: ChunkStore::with_max_disk_usage(1073741824) }
  }

  pub fn handle_get(&self, name: NameType) ->Result<Action, InterfaceError> {
    let data = self.chunk_store_.get(name);
    if data.len() == 0 {
      return Err(From::from(ResponseError::NoData));
    }
    Ok(Action::Reply(data))
  }

  pub fn handle_put(&mut self, data : Vec<u8>) ->Result<Action, InterfaceError> {
    let mut data_name : NameType;
    let mut d = cbor::Decoder::from_bytes(&data[..]);
    let payload: maidsafe_types::Payload = d.decode().next().unwrap().unwrap();
    match payload.get_type_tag() {
      maidsafe_types::PayloadTypeTag::StructuredData => {
        data_name = payload.get_data::<maidsafe_types::StructuredData>().name();
      }
       _ => return Err(From::from(ResponseError::InvalidRequest))
    }
    // the type_tag needs to be stored as well, ChunkStore::put is overwritable
    self.chunk_store_.put(data_name, data);
    return Err(InterfaceError::Abort);
  }

  pub fn retrieve_all_and_reset(&mut self) -> Vec<routing::node_interface::RoutingNodeAction> {
       let names = self.chunk_store_.names();
       let mut actions = Vec::with_capacity(names.len());
       for name in names {
            let data = self.chunk_store_.get(name.clone());
            actions.push(routing::node_interface::RoutingNodeAction::Refresh {
                content: Box::new(VersionHandlerSendable::new(name, data)),
            });
       }
       self.chunk_store_ = ChunkStore::with_max_disk_usage(1073741824);
       actions
  }

}

#[cfg(test)]
mod test {
 use cbor;
 use maidsafe_types;
 use super::*;
 use maidsafe_types::*;
 use routing::types::*;
 use routing::error::InterfaceError;
 use routing::NameType;
 use routing::sendable::Sendable;

 #[test]
 fn handle_put_get() {
    let mut version_handler = VersionHandler::new();
    let name = NameType([3u8; 64]);
    let owner = NameType([4u8; 64]);
    let mut value = Vec::new();
    value.push(vec![NameType([5u8; 64]), NameType([6u8; 64])]);
    let sdv = StructuredData::new(name, owner, value);
    let payload = Payload::new(PayloadTypeTag::StructuredData, &sdv);
    let mut encoder = cbor::Encoder::from_memory();
    let encode_result = encoder.encode(&[&payload]);
    assert_eq!(encode_result.is_ok(), true);

    let put_result = version_handler.handle_put(array_as_vector(encoder.as_bytes()));
    assert_eq!(put_result.is_err(), true);
    match put_result.err().unwrap() {
        InterfaceError::Abort => assert_eq!(true, true),
        _ => assert_eq!(true, false),
    }

    let data_name = NameType::new(sdv.name().0);
    let get_result = version_handler.handle_get(data_name);
    assert_eq!(get_result.is_err(), false);
    match get_result.ok().unwrap() {
        Action::SendOn(_) => panic!("Unexpected"),
        Action::Reply(x) => {
                let mut d = cbor::Decoder::from_bytes(x);
                let obj_after: Payload = d.decode().next().unwrap().unwrap();
                assert_eq!(obj_after.get_type_tag(), PayloadTypeTag::StructuredData);
                let sdv_after = obj_after.get_data::<maidsafe_types::StructuredData>();
                assert_eq!(sdv_after.name(), NameType([3u8;64]));
                assert_eq!(sdv_after.owner().unwrap(), NameType([4u8;64]));
                assert_eq!(sdv_after.get_value().len(), 1);
                assert_eq!(sdv_after.get_value()[0].len(), 2);
                assert_eq!(sdv_after.get_value()[0][0], NameType([5u8;64]));
                assert_eq!(sdv_after.get_value()[0][1], NameType([6u8;64]));
            }
        }
    }

    #[test]
    fn version_handler_sendable_serialisation() {
        let obj_before = super::VersionHandlerSendable::new(NameType([1u8;64]), vec![2,3,45,5]);

        let mut e = cbor::Encoder::from_memory();
        e.encode(&[&obj_before]).unwrap();

        let mut d = cbor::Decoder::from_bytes(e.as_bytes());
        let obj_after: super::VersionHandlerSendable = d.decode().next().unwrap().unwrap();

        assert_eq!(obj_before, obj_after);
    }


}
