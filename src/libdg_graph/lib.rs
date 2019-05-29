// Copyright 2019 Karl Sundequist Blomdahl <karl.sundequist.blomdahl@gmail.com>
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
#![feature(test)]

extern crate dg_cuda;
extern crate dg_utils;
extern crate fnv;
#[macro_use] extern crate lazy_static;
extern crate libc;
extern crate serde;
extern crate serde_json;
extern crate test;

mod factories;
mod graph_def;
mod layer;
mod optimizers;
mod layers;
mod graph;
mod session;
mod graph_loader;

pub use session::Session;
pub use graph_loader::{GraphLoader, GraphLoaderError};