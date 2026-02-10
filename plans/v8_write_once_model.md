Support for write(2) is handled in a special non-POSIX way similar to how files are uploaded to an object store.
Use the write-once semantics: files opened for writing are not visible to others until after close() is called.
The process for writing to a file consists of:
1. open(O_WRONLY, O_CREAT | O_EXCL | O_APPEND)
2. One or more write() calls
3. close()

Avoid the need to keep a session state on the server side if possible. In response to the open() call,
the server should create a zero-length file with the requested name. The write() API call should include
the full path to the writable file, so the server does not need to keep track of that detail. Think of
a way to mark the incomplete file as being opened for writing; for example, set an xattr named "fs123.write_session_active"
with a value of "true", and only allow writes to files with that xattr in place.
All write() calls by the client should be appended to the end of the file, so we don't need to keep
track of a position offset within the file. When the server receives the close() request, it should remove the fs123.write_session_active xattr from the file.
