#!/usr/bin/env python3
"""Static checks for standalone Dock packaging and its startup policy."""
import json
import pathlib
import tomllib

root = pathlib.Path(__file__).resolve().parents[1]
manifest = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))
members = manifest['workspace']['members']
assert 'crates/aura-explorer' not in members and 'crates/aura-service' not in members
assert not (root / 'crates/aura-explorer').exists()
assert not (root / 'crates/aura-service').exists()
entries = json.loads((root / 'packaging/bundle.json').read_text(encoding='utf-8'))['entries']
bins = {e['name']: e['source'] for e in entries if e['name'].lower().endswith('.exe')}
assert bins == {
    'AuraDock.exe': {'kind':'cargo','package':'aura-dock','bin':'aura-dock'},
    'widgets.exe': {'kind':'cargo','package':'aura-dock','bin':'widgets'},
}
assert not any('WindowHook' in e['name'] for e in entries)
for p in ['packaging/windows/aura.iss', 'packaging/windows/install.ps1']:
    text = (root / p).read_text(encoding='utf-8-sig')
    for name in bins: assert name in text, (p,name)
assert 'mod service_start;' not in (root/'crates/aura-fences/src/main.rs').read_text(encoding='utf-8')
assert 'ensure_own_pin' not in (root/'crates/aura-dock/src/dock.rs').read_text(encoding='utf-8')
# Removed notes widget must not keep reading or editing its old data file.
for relative in ('widgets/hub.rs', 'widgets/paint.rs', 'desktop_system.rs', 'dock_widgets.rs'):
    text = (root / 'crates/aura-dock/src' / relative).read_text(encoding='utf-8')
    assert all(retired not in text for retired in ('notes_path(', 'ToolKind::Notes', 'WAct::Notes', 'Kind::Notes'))
assert not (root / 'crates/aura-dock/src/desktop_notes.rs').exists()
assert all(e['name'] != 'Fences.exe' for e in entries)
print('Standalone Dock bundle checks passed (static only).')
