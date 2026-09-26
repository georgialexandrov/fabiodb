#!/usr/bin/env python3
"""Stands in for `ssh -N -L 127.0.0.1:LOCAL:HOST:PORT … destination` in tests:
forwards the local port to HOST:PORT directly. Appends its arguments to
$FAKE_SSH_LOG. A destination of fail.example fails like a refused key."""
import os, socket, sys, threading

args = sys.argv[1:]
with open(os.environ["FAKE_SSH_LOG"], "a") as log:
    log.write(" ".join(args) + "\n")
if args[-1].endswith("fail.example"):
    sys.stderr.write("git@fail.example: Permission denied (publickey).\n")
    sys.exit(255)

_, local, host, port = args[args.index("-L") + 1].split(":")
server = socket.socket()
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", int(local)))
server.listen()

def pipe(a, b):
    try:
        while data := a.recv(65536):
            b.sendall(data)
    except OSError:
        pass
    finally:
        a.close(); b.close()

while True:
    client, _ = server.accept()
    upstream = socket.create_connection((host, int(port)))
    threading.Thread(target=pipe, args=(client, upstream), daemon=True).start()
    threading.Thread(target=pipe, args=(upstream, client), daemon=True).start()
