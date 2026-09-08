#!/usr/bin/env python3
"""Throwaway assertion: descriptors alone do not prove local platform content."""
import json
import pathlib
import subprocess
import sys

layout, machine, *expected = sys.argv[1:]
layout = pathlib.Path(layout)

def blob(digest):
    return json.loads((layout / 'blobs' / digest.replace(':', '/')).read_text())

index = json.loads((layout / 'index.json').read_text())
root = next(m for m in index['manifests']
            if m.get('annotations', {}).get('org.opencontainers.image.ref.name') == 'multi')
index = blob(root['digest'])
assert sorted(m['platform']['architecture'] for m in index['manifests']) == ['amd64', 'arm64']
contents = set(subprocess.check_output([
    'docker', 'exec', machine, 'ctr', '--address',
    '/var/run/docker/containerd/containerd.sock', '--namespace', 'moby',
    'content', 'ls', '-q'], text=True).splitlines())
assert root['digest'] in contents, 'index missing from Docker containerd store'
result = {'machine': machine, 'index': root['digest'], 'platforms': {}}
for desc in index['manifests']:
    arch = desc['platform']['architecture']
    manifest = blob(desc['digest'])
    required = {desc['digest'], manifest['config']['digest']}
    required.update(layer['digest'] for layer in manifest['layers'])
    missing = sorted(required - contents)
    result['platforms'][arch] = {'manifest': desc['digest'], 'complete': not missing,
                                'missing_blobs': missing}
    if arch in expected:
        assert not missing, f'{machine}: {arch} content missing: {missing}'
print(json.dumps(result, indent=2))
