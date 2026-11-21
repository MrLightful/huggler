import sdk = require("@huggler/sdk")
import assert = require("node:assert/strict")

assert.equal(typeof sdk.Huggler.open, "function")
assert.equal(sdk.threadId("example"), "example")
