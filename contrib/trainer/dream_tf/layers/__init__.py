# Copyright (c) 2019 Karl Sundequist Blomdahl <karl.sundequist.blomdahl@gmail.com>
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in all
# copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.

import tensorflow as tf

from ..ffi.libdg_go import get_num_features

""" The total number of input features """
NUM_FEATURES = get_num_features()


def normalize_constraint(x):
    """ Returns a constraint that set each output vector to `tf.norm(x) = 1` """
    out_dims = x.shape[-1]
    x_f = tf.reshape(x, (-1, out_dims))
    n = tf.norm(x_f, axis=0)
    x_n = x_f / (tf.sqrt(tf.cast(out_dims, tf.float32)) * tf.reshape(n, (1, out_dims)))

    return tf.reshape(x_n, x.shape)


def unit_constraint(x):
    """ Return a constraint that clip `x` to the range [0, 1] """
    return tf.clip_by_value(x, 0.0, 1.0)


def conv2d(x, weights):
    """ Shortcut for `tf.nn.conv2d` """
    return tf.nn.conv2d(x, tf.cast(weights, tf.float16), (1, 1, 1, 1), 'SAME', True, 'NHWC')