#!/bin/sh
# Runs the right-click row plan's tests on Linux.
#
# context_menu_common.rs cannot be compiled here as it stands: it reaches the Shell through
# aura-windows, which is Windows-only. This bench keeps the module's own logic — the row
# plan and its tests — and swaps the three things it borrows from elsewhere for stand-ins:
# aura-core's text lookup, the Shell's option type, and MenuRow itself. The module is copied
# in fresh every run, so the tests always describe the code in the tree.
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
src="$root/crates/aura-explorer/src/context_menu_common.rs"

cp "$src" "$here/src/common.rs"
python3 - "$here/src/common.rs" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
# The module's own imports, pointed at the bench's stand-ins.
s = s.replace("use super::{Mode, Request};", "use crate::{archive_among, request::{Mode, Request}};")
s = s.replace("use aura_windows::{", "use crate::aura_windows::{")
s = s.replace("aura_core::i18n::text", "crate::aura_core::i18n::text")
s = s.replace("aura_windows::shell::menu::", "crate::aura_windows::shell::menu::")
# The archive helper lives at this crate's root in the app; here it is a stand-in.
s = s.replace("crate::archive::archive_among", "archive_among")
# Tests read identities out of a plan they own, so the slice's lifetime is named.
s = s.replace("fn commands(rows: &[MenuRow<'_>]) -> Vec<&str> {", "fn commands<'a>(rows: &'a [MenuRow<'a>]) -> Vec<&'a str> {")
open(p, "w").write(s)
PY
cd "$here" && cargo test --quiet "$@"
