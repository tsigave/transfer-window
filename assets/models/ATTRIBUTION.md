# Shape-model attribution

## NASA/JPL-Caltech Martian moons

The bundled `phobos.glb` and `deimos.glb` files are redistributed without modification from
NASA Science's [Phobos](https://science.nasa.gov/resource/phobos-mars-moon-3d-model/) and
[Deimos](https://science.nasa.gov/resource/deimos-mars-moon-3d-model/) 3D resource pages.
Both source pages credit NASA/JPL-Caltech. The models contain measured irregular geometry and
embedded spacecraft-image surface maps; no NASA identifier or logo is included in the bundled
files.

NASA's public [3D Resources repository](https://github.com/nasa/NASA-3D-Resources) describes its
assets as free and without copyright. Redistribution remains subject to the
[NASA media usage guidelines](https://www.nasa.gov/nasa-brand-center/images-and-media/): preserve
the source credit, do not imply NASA endorsement, and do not use NASA identifiers as branding.

Bundled asset integrity:

- `phobos.glb` — 16,449 vertices; SHA-256
  `757712101e4fbcf9527942ee664a3bc6dfe3ab460466b760f4bf51ee85dbd8d3`.
- `deimos.glb` — 16,649 vertices; SHA-256
  `5d5da763970acffe88aac26d3c3bd0fa66c5bef251d1bb4bfe2ae5fd723507e4`.

The renderer replaces the source materials with non-metallic, high-roughness regolith settings
while retaining the original geometry, UV coordinates, normals, and embedded color textures.
