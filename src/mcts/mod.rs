// Copyright 2017 Karl Sundequist Blomdahl <karl.sundequist.blomdahl@gmail.com>
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

mod param;
mod dirichlet;
mod spin;
mod tree;

use ordered_float::OrderedFloat;
use rand::{thread_rng, Rng};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use go::{symmetry, Board, Color};
use mcts::param::*;
use nn::{self, Network, Workspace};
use util::b85;

/// Mapping from 1D coordinate to letter used to represent that coordinate in
/// the SGF file format.
const SGF_LETTERS: [char; 26] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k',
    'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v',
    'w', 'x', 'y', 'z'
];

pub enum GameResult {
    Resign(String, Board, Color, f32),
    Ended(String, Board)
}

/// An abstraction that hides the exact details of how a neural network forward
/// pass is implemented. There are two main implementation `ImmediateForward` and
/// `RemoteForward`, where the former performs the forward pass immediate with no
/// batching and the second forwards it to another worker threads that performs
/// the actual work.
/// 
/// All methods appears to be synchronous but may sleep due to communication with
/// other threads.
trait Forwarder {
    /// Perform a forward pass of a neural network with the given features
    /// and returns the value and policy.
    /// 
    /// # Arguments
    /// 
    /// * `features` -
    /// 
    fn forward(&mut self, features: Box<[f32]>) -> (f32, Box<[f32]>);
}

/// An implementation of `Forwarder` that performs the forward pass immedietly on
/// a local `nn::Workspace` with batch size `1`.
struct ImmediateForward<'a> {
    workspace: Workspace<'a>
}

impl<'a> ImmediateForward<'a> {
    fn new(network: &'a Network) -> ImmediateForward<'a> {
        ImmediateForward {
            workspace: network.get_workspace(1)
        }
    }
}

impl<'a> Forwarder for ImmediateForward<'a> {
    fn forward(&mut self, features: Box<[f32]>) -> (f32, Box<[f32]>) {
        let (values, policies) = nn::forward(&mut self.workspace, &vec! [features]);

        (values[0], policies[0].clone())
    }
}

/// An implementation of `Forwarder` that sends the received features over a
/// channel and relies on the remote endpoint performing the forward
/// pass (presumably with some batching).
struct RemoteForward {
    remote: Sender<(Box<[f32]>, Sender<(f32, Box<[f32]>)>)>
}

impl Forwarder for RemoteForward {
    fn forward(&mut self, features: Box<[f32]>) -> (f32, Box<[f32]>) {
        let (sender, receiver) = channel();

        self.remote.send((features, sender)).unwrap();
        let (value, policy) = receiver.recv().unwrap();

        (value, policy)
    }
}

/// Performs a forward pass through the neural network for the given board
/// position using a random symmetry to increase entropy.
/// 
/// # Arguments
/// 
/// * `workspace` - the workspace to use during the forward pass
/// * `board` - the board position
/// * `color` - the current player
/// 
fn forward<C, A>(agent: &mut A, board: &Board, color: Color) -> (f32, Box<[f32]>)
    where C: Param, A: Forwarder
{
    lazy_static! {
        static ref SYMM: Vec<symmetry::Transform> = vec! [
            symmetry::Transform::Identity,
            symmetry::Transform::FlipLR,
            symmetry::Transform::FlipUD,
            symmetry::Transform::Transpose,
            symmetry::Transform::TransposeAnti,
            symmetry::Transform::Rot90,
            symmetry::Transform::Rot180,
            symmetry::Transform::Rot270,
        ];
    }

    // pick a random transformation to apply to the features. This is done
    // to increase the entropy of the game slightly and to ensure the engine
    // learns the game is symmetric (which should help generalize)
    let t = *thread_rng().choose(&SYMM).unwrap();
    let mut features = board.get_features(color);

    symmetry::apply(&mut features, t);

    // run a forward pass through the network using this transformation
    // and when we are done undo it using the opposite.
    let (value, mut policy) = agent.forward(features);

    symmetry::apply(&mut policy, t.inverse());

    // replace any invalid moves in the suggested policy with -Inf, while keeping
    // the pass move (361) untouched so that there is always at least one valid
    // move.
    for i in 0..361 {
        let (x, y) = (tree::X[i] as usize, tree::Y[i] as usize);

        if !board.is_valid(color, x, y) {
            policy[i] = ::std::f32::NEG_INFINITY;
        }
    }

    // get ride of symmetric moves, this is mostly useful for the opening.
    // Once we are past the first ~7 moves the board is usually sufficiently
    // asymmetric for this to turn into a no-op.
    //
    // we skip the first symmetry because it is the identity symmetry, which
    // is always a symmetry for any board.
    for &t in &SYMM[1..8] {
        if !symmetry::is_symmetric(board, t) {
            continue;
        }

        // figure out which are the useful vertices by eliminating the
        // symmetries from the board.
        let mut visited = [false; 368];

        for i in 0..361 {
            let j = t.apply(i);

            if i != j && !visited[i] {
                visited[i] = true;
                visited[j] = true;

                let src = ::std::cmp::max(i, j);
                let dst = ::std::cmp::min(i, j);

                if policy[src].is_normal() {
                    assert!(policy[dst].is_normal());

                    policy[dst] += policy[src];
                    policy[src] = ::std::f32::NEG_INFINITY;
                }
            }
        }
    }

    // renormalize the policy so that it sums to one after all the pruning that
    // we have performed.
    let policy_sum: f32 = policy.iter().filter(|p| p.is_normal()).sum();

    if policy_sum > 1e-4 {  // do not divide by zero
        let policy_recip = policy_sum.recip();

        for i in 0..362 {
            policy[i] *= policy_recip;
        }
    }

    (value, policy)
}

/// The shared variables between the master and each worker thread in the `predict` function.
#[derive(Clone)]
struct ThreadContext<E: tree::Value + Clone> {
    /// The root of the monte carlo tree.
    root: Arc<UnsafeCell<tree::Node<E>>>,

    /// The number of probes that still needs to be done into the tree.
    remaining: Arc<AtomicIsize>,

    /// The initial board position at the root the tree.
    starting_point: Board,

    /// The channel to use when communicating features to the cuDNN worker thread.
    sender: Sender<(Box<[f32]>, Sender<(f32, Box<[f32]>)>)>
}

unsafe impl<E: tree::Value + Clone> Send for ThreadContext<E> { }

/// Predicts the _best_ next move according to the given neural network when applied
/// to a monte carlo tree search.
/// 
/// # Arguments
/// 
/// * `network` -
/// * `starting_point` -
/// * `starting_color` -
/// 
pub fn predict<C: Param + Clone + 'static, E: tree::Value + Clone + 'static>(
    network: &Network,
    starting_point: &Board,
    starting_color: Color
) -> (f32, usize, usize, Box<[f32]>)
{
    assert_eq!(C::iteration_limit() % C::batch_size(), 0);
    assert_eq!(C::thread_count() % C::batch_size(), 0);

    // add some dirichlet noise to the root node of the search tree in order to increase
    // the entropy of the search and avoid overfitting to the prior value
    let mut immediate = ImmediateForward::new(network);
    let (_, mut policy) = forward::<C, ImmediateForward>(&mut immediate, starting_point, starting_color);
    dirichlet::add::<C>(&mut policy, 0.03);

    // perform exactly NUM_ITERATIONS probes into the search tree
    let (sender, receiver) = channel();
    let context: ThreadContext<E> = ThreadContext {
        root: Arc::new(UnsafeCell::new(tree::Node::new(starting_color, policy))),
        remaining: Arc::new(AtomicIsize::new(C::iteration_limit() as isize)),
        starting_point: starting_point.clone(),
        sender: sender
    };

    let handles = (0..C::thread_count()).map(|_| {
        let context = context.clone();

        thread::spawn(move || {
            let mut remote = RemoteForward { remote: context.sender };

            while context.remaining.fetch_sub(1, Ordering::SeqCst) > 0 {
                let mut board = context.starting_point.clone();
                let trace = unsafe { tree::probe::<C, E>(&mut *context.root.get(), &mut board) };

                if let Some(&(_, color, _)) = trace.last() {
                    let next_color = color.opposite();
                    let (value, policy) = forward::<C, RemoteForward>(&mut remote, &board, next_color);

                    unsafe {
                        tree::insert::<C, E>(&trace, next_color, value, policy);
                    }
                }
            }
        })
    }).collect::<Vec<JoinHandle<()>>>();

    // process the requests from all worker threads in the main thread, we keep
    // an independent count instead of relying on `remaining` to avoid race-conditions
    // between when we check the loop invariant, when the workers decrease the
    // counter, and when the workers receive the response from the network.
    let mut workspace_b = network.get_workspace(C::batch_size());
    let batch_size = C::batch_size();

    for _ in 0..(C::iteration_limit() / batch_size) {
        // collect a full batch worth of features from the workers
        let mut features_list = vec! [];
        let mut sender_list = vec! [];

        for _ in 0..batch_size {
            let (features, sender) = receiver.recv().unwrap();

            features_list.push(features);
            sender_list.push(sender);
        }

        // process the features and the send them back to the worker who
        // sent it using the OneShot channel.
        let (values, policies) = nn::forward(&mut workspace_b, &features_list);

        for (i, policy) in policies.into_iter().enumerate() {
            sender_list[i].send((values[i], policy)).unwrap();
        }
    }

    // wait for all threads to finish their work and then terminate
    // with some additional information
    for handle in handles.into_iter() { handle.join().unwrap(); }

    unsafe {
        let root = &*context.root.get();
        let (value, index) = root.best();
        let (_, prior_index) = root.prior();
        let policy = root.softmax();

        #[cfg(feature = "trace-mcts")]
        println!("{}", tree::to_sgf::<C, E>(root, starting_point));

        (value, index, prior_index, policy)
    }
}

/// A variant of `predict` that does not perform any search and only uses the neural network.
/// 
/// # Arguments
/// 
/// * `network` -
/// * `starting_point` -
/// * `starting_color` -
/// 
#[allow(dead_code)]
pub fn predict_policy<C: Param>(
    network: &Network,
    starting_point: &Board,
    starting_color: Color
) -> (f32, usize, usize, Box<[f32]>)
{
    let mut immediate = ImmediateForward::new(network);
    let (value, policy) = forward::<C, ImmediateForward>(&mut immediate, starting_point, starting_color);
    let policy_index = (0..362).max_by_key(|&i| OrderedFloat(policy[i])).unwrap();
    let mut softmax = vec! [0.0f32; 362];
    softmax[policy_index] = 1.0;

    (value, policy_index, policy_index, softmax.into_boxed_slice())
}

/// Play a game against the engine and return the result of the game.
/// 
/// # Arguments
/// 
/// * `workspace` - the neural network workspace to use during evaluation
/// 
pub fn self_play(network: &Network) -> GameResult {
    let mut board = Board::new();
    let mut sgf = String::new();
    let mut current = Color::Black;
    let mut pass_count = 0;
    let mut count = 0;

    // limit the maximum number of moves to `2 * 19 * 19` to avoid the
    // engine playing pointless capture sequences at the end of the game
    // that does not change the final result.
    while count < 722 {
        let (value, index, prior_index, policy) = if current == Color::Black {
            predict::<Standard, tree::PUCT>(network, &board, current)
        } else {
            predict::<Standard, tree::PUCT>(network, &board, current)
            //predict_policy::<Standard>(network, &board, current)
        };

        debug_assert!(-1.0 <= value && value <= 1.0);
        debug_assert!(index < 362);

        if value < -0.9 {  // resign the game if the evaluation looks bad
            sgf += &format!(";{}[]", current);

            return GameResult::Resign(sgf, board, current.opposite(), -value);
        } else {
            if index == 361 {  // passing move
                sgf += &format!(";{}[]P[{}]", current, b85::encode(&policy));
                pass_count += 1;

                if pass_count >= 2 {
                    return GameResult::Ended(sgf, board)
                }
            } else {
                let (x, y) = (tree::X[index] as usize, tree::Y[index] as usize);
                let (px, py) = if prior_index == 361 {
                    (19, 19)
                } else {
                    (tree::X[prior_index] as usize, tree::Y[prior_index] as usize)
                };

                sgf += &format!(";{}[{}{}]P[{}]TR[{}{}]",
                    current,
                    SGF_LETTERS[x], SGF_LETTERS[y],
                    b85::encode(&policy),
                    SGF_LETTERS[px], SGF_LETTERS[py]
                );
                pass_count = 0;

                board.place(current, x, y);
            }
        }

        current = current.opposite();
        count += 1;
    }

    GameResult::Ended(sgf, board)
}
