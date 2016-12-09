// Copyright 2016 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net
// Commercial License, version 1.0 or later, or (2) The General Public License
// (GPL), version 3, depending on which licence you accepted on initial access
// to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project
// generally, you agree to be bound by the terms of the MaidSafe Contributor
// Agreement, version 1.0.
// This, along with the Licenses can be found in the root directory of this
// project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network
// Software distributed under the GPL Licence is distributed on an "AS IS"
// BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
// implied.
//
// Please review the Licences for the specific language governing permissions
// and limitations relating to use of the SAFE Network Software.

use app::App;
use app::ffi::helper::send_sync;
use app::object_cache::DirHandle;
use routing::XOR_NAME_LEN;
use std::os::raw::c_void;
use std::slice;
use util::ffi::{catch_unwind_cb, u8_vec_to_ptr};

/// Encrypt entry key of a dir.
#[no_mangle]
pub unsafe extern "C" fn dir_encrypt_entry_key(app: *const App,
                                               dir_h: DirHandle,
                                               input_ptr: *const u8,
                                               input_len: usize,
                                               user_data: *mut c_void,
                                               o_cb: extern "C" fn(*mut c_void,
                                                                   i32,
                                                                   *mut u8,
                                                                   usize,
                                                                   usize)) {
    catch_unwind_cb(user_data, o_cb, || {
        let input = slice::from_raw_parts(input_ptr, input_len).to_vec();

        send_sync(app, user_data, o_cb, move |context| {
            let dir = context.object_cache().get_dir(dir_h)?;
            let output = dir.enc_entry_key(&input)?;
            Ok(u8_vec_to_ptr(output))
        })
    })
}

/// Encrypt entry value of a dir.
#[no_mangle]
pub unsafe extern "C" fn dir_encrypt_entry_value(app: *const App,
                                                 dir_h: DirHandle,
                                                 input_ptr: *const u8,
                                                 input_len: usize,
                                                 user_data: *mut c_void,
                                                 o_cb: extern "C" fn(*mut c_void,
                                                                     i32,
                                                                     *mut u8,
                                                                     usize,
                                                                     usize)) {
    catch_unwind_cb(user_data, o_cb, || {
        let input = slice::from_raw_parts(input_ptr, input_len).to_vec();

        send_sync(app, user_data, o_cb, move |context| {
            let dir = context.object_cache().get_dir(dir_h)?;
            let output = dir.enc_entry_value(&input)?;
            Ok(u8_vec_to_ptr(output))
        })
    })
}

/// Extract name and type tag from a dir.
#[no_mangle]
pub unsafe extern "C" fn dir_extract_name_and_type_tag(app: *const App,
                                                       dir_h: DirHandle,
                                                       user_data: *mut c_void,
                                                       o_cb: extern "C" fn(*mut c_void,
                                                                           i32,
                                                                           [u8; XOR_NAME_LEN],
                                                                           u64)) {
    catch_unwind_cb(user_data, o_cb, || {
        send_sync(app, user_data, o_cb, move |context| {
            let dir = context.object_cache().get_dir(dir_h)?;
            Ok((dir.name.0, dir.type_tag))
        })
    })
}
