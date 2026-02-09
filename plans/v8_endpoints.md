Create a new protocol version number 8.0 ("v8"). The first big change for v8
is to go from single-letter handler names to multi-letter names that match the kernel function. For example, here are some mappings between single-letter functions and their new v8 counterparts:

a -> stat
d -> readdir
f -> read
l -> readlink
s -> statvfs
x -> getxattr and listxattr

Note that some v7.3 functions like "x" may now correspond to multiple v8 functions. The name of the function should match the equivalent FUSE function call that implements the function.
