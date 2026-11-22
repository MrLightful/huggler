"""Advance the subscription only when the Python reader receives an event."""

import asyncio

from ._native import HugglerError


class EventStream:
    """Async iterator over one Huggler thread's events."""

    def __init__(self, stream):
        self._stream = stream
        self._lock = asyncio.Lock()

    async def next(self):
        """Read the next event, raising HugglerError when the runtime closes."""
        async with self._lock:
            event = await self._stream.peek()
            self._stream.consume()
            return event

    def __aiter__(self):
        return self

    async def __anext__(self):
        try:
            return await self.next()
        except HugglerError as error:
            if error.code == "closed":
                raise StopAsyncIteration from None
            raise
