"""Mount the native peer and guest routes in a host-owned ASGI server."""

from urllib.parse import quote, quote_from_bytes

# Keep one extra byte so Axum can recognize an oversized JSON body.
_MAX_BUFFERED_BODY = 2 * 1024 * 1024 + 1


def asgi_app(router):
    """Return an HTTP ASGI app backed by a Huggler RemoteRouter."""

    async def app(scope, receive, send):
        if scope["type"] == "lifespan":
            while True:
                message = await receive()
                if message["type"] == "lifespan.startup":
                    await send({"type": "lifespan.startup.complete"})
                elif message["type"] == "lifespan.shutdown":
                    await send({"type": "lifespan.shutdown.complete"})
                    return
        if scope["type"] != "http":
            raise ValueError("Huggler peer and guest routes require HTTP")
        body = bytearray()
        while True:
            message = await receive()
            if message["type"] == "http.disconnect":
                return
            remaining = _MAX_BUFFERED_BODY - len(body)
            body.extend(message.get("body", b"")[:remaining])
            if len(body) == _MAX_BUFFERED_BODY or not message.get("more_body", False):
                break
        raw_path = scope.get("raw_path")
        if raw_path is None:
            path = quote(scope["path"])
        else:
            path = quote_from_bytes(raw_path, safe="/%")
        status, headers, response = await router.request(
            scope["method"], path, scope["headers"], bytes(body)
        )
        await send({"type": "http.response.start", "status": status, "headers": headers})
        await send({"type": "http.response.body", "body": response})

    return app
