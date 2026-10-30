file-lock-io = { $path }: { $ext_err_txt }
file-lock-relative-path = { $path } is a relative path; a lock is only ever taken at an absolute one
file-lock-not-regular = the lock { $path } is not a regular file, so it cannot be locked
file-lock-timeout = the lock { $path } is still held after { $waited_ms }ms: another ctxloom, taskloom or ltk process holds it; try again once it exits
file-lock-replaced = the lock { $path } was replaced { $attempts } times while being taken: try again
