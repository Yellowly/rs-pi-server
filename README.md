# RSPI Process Manager
Command line application that aims to provide a means of running and managing simple shell commands on a Raspberry PI server remotely.

This functions similarly to Secure Shell (SSH), but with the goal of allowing clients to manage long-running processes as well.

# RSPI Server
This repo is the server side of the RSPI Process Manager. See the client-side repo [here]([http://example.com](https://github.com/Yellowly/rs-pi-client)

# Usage
Before running the executable for this, make sure you define the following environment variables:
- RSPI_SERVER_ADDR = Socket address to bind to, ie. "127.0.0.1:8080"
- RSPI_SERVER_HASHKEY = An unsigned 64-bit integer used to encrypt data sent between client and server
- RSPI_SERVER_PASS = Any string less than 64 bytes long that the client must send as their first message to connect to the server

Then, simply run the executable
