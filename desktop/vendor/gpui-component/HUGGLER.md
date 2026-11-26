# GPUI Component 0.5.1

Source is the published Apache-2.0 crate at revision
`0f0ab35233212f8f3277028995caf0c41e13ee6c`. Huggler added inline elements to
multiline input. Matching ranges keep their original UTF-8 bytes while sharing
one layout with native text, selection, cursor movement, and scrolling. The
option is off for inputs that supply no inline elements. Explicit inline edits
can also start a standalone undo group.

The [standalone patch](../gpui-component-inline-input.patch) records the full
source delta against the published crate. To verify or carry the patch forward,
unpack `gpui-component-0.5.1.crate` and run `git apply --check` with the
absolute patch path from the unpacked crate directory. Cargo builds the local
source tree directly.
