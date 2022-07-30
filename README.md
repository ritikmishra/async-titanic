# async-titanic

_an exploration into how little heap we need to use async rust_

I made this async executor to poke around at async without using a lot of heap allocation. 

The example uses sockets + mpsc, but maybe a design not too different from this one could be used on an embedded device where you know ahead of time exactly what peripherals you will be using.

There is a mechanism to store unnameable futures (i.e the return type of `async fn`s) statically, as well as a waker implementation that does not strictly require an allocator.
