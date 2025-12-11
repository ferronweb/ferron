---
title: Server concurrency models, explained
description: Learn about different server concurrency models and how they affect performance.
date: 2025-12-11 09:28:00
cover: ./covers/server-concurrency-models-explained.png
author: Dorian Niemiec
---

Network servers (such as web servers and database servers) often need to handle multiple concurrent connections from clients. Because of this, they implement a concurrency model. A concurrency model is a way a server can handle many connections at once.

In this post, you'll learn about different concurrency models (such as ones utilizing blocking and non-blocking I/O).

## Single-threaded process with blocking I/O

A network server that uses blocking I/O and runs in a single thread is the simplest design, but it can handle only one connection at a time - no concurrency. As a result, it is extremely vulnerable to Slowloris-style attacks, where an attacker keeps many connections open for as long as possible. Because the server cannot process other connections while waiting, it can be taken down easily.

Examples of such servers can include many network servers created when learning networking.

## Many processes or threads with blocking I/O

Using many processes or threads with blocking I/O is more complex than a single-threaded design but still simpler than non-blocking, event-driven architectures. This model supports concurrent connections by assigning each connection to a worker thread or process. However, the number of connections that can be handled at once is limited by the size of the thread or process pool. This limit also makes the server susceptible to Slowloris-style attacks, which can exhaust available workers.

Examples of such servers include:

- Apache HTTP Server (pre-fork and worker MPMs)
  - Pre-fork MPM utilizes multiple processes
  - Worker MPM utilizes multiple threads
- PHP-FPM (pre-fork model, which utilizes multiple processes)

## Non-blocking event-driven I/O

An event-driven server using non-blocking I/O is the most complex to implement from scratch, but it typically performs the best - especially when combined with many processes or threads. Unlike blocking designs, where each operation must complete before another begins, non-blocking I/O allows a single thread or process to manage many operations concurrently. This approach removes the strict dependency on the number of workers and can theoretically support a very large number of connections. It also makes Slowloris-style attacks far less effective.

There are many examples of such servers, such as:

- NGINX (multiple worker processes, non-blocking I/O)
- Servers built with Node.js (non-blocking I/O)
- Redis (non-blocking I/O)
- Caddy (multi-threading, non-blocking I/O)
- Ferron (multi-threading, non-blocking I/O)

## Conclusion

There are many different concurrency models, from ones utilizing blocking I/O to ones utilizing non-blocking I/O. Choosing the right concurrency model can also decide how many connections a network server can handle at once.
