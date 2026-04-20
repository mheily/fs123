Protocol v8 adds support for writable filesystems. Not every backend will support writing. Protocol 7.3 clients and servers will not support the new functions.

Add support for the following FUSE functions to the Backend trait:
* mkdir, rmdir
* chmod, chown, utimens
* symlink, link, unlink, rename
* setxattr, removexattr
* access
* open, write, close

The default implementation for each new Backend method is to return ENOSYS as an errno.
Use RESTful API principles to select the appropriate HTTP VERB (PUT, PATCH, DELETE, etc).
In this first coding session, here are the goals: 
* Modify the FileBackend struct to implement all new FUSE functions except for open(), write(), and close().
* Limit the scope of changes to protocol v8-only functions. Do not make any changes to protocol 7.3 code.
* Add a default WritableBackend trait to the DatabaseBackend so that it compiles, but do not implement any of the methods.

