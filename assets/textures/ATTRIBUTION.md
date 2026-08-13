# Texture and sky-map attribution

## Solar System Scope

The texture files in this directory are adapted/distributed from the
[Solar System Scope texture pack](https://www.solarsystemscope.com/textures/), created by
Solar System Scope / INOVE.

They are licensed under the
[Creative Commons Attribution 4.0 International License](https://creativecommons.org/licenses/by/4.0/).
The source page explicitly permits use, adaptation, and sharing for any purpose, including
commercial use. Solar System Scope notes that the pack is based on NASA elevation and imagery
data, with color and unmapped areas adjusted by the texture authors.

Bundled 2K assets:

- `sun.jpg`
- `mercury.jpg`
- `venus_atmosphere.jpg`
- `earth_daymap.jpg`
- `earth_clouds.jpg`
- `earth_nightmap.jpg`
- `moon.jpg`
- `mars.jpg`
- `jupiter.jpg`
- `saturn.jpg`
- `saturn_ring.png`
- `uranus.jpg`
- `neptune.jpg`
- `ceres.jpg` (fictional completion)
- `haumea.jpg` (fictional)
- `makemake.jpg` (fictional)
- `eris.jpg` (fictional)

No NASA logo or identifier is included. The texture pack must be credited as “Solar System
Scope / INOVE, CC BY 4.0” in distributions that use these files.

The Earth cloud and night maps were retrieved through their Wikimedia Commons mirrors;
their file pages identify Solar System Scope as the author and retain the same CC BY 4.0
license. The renderer uses the night map only on the hemisphere facing away from the simulated
Sun. The cloud layer is a visual atmosphere approximation, not a live weather dataset.

## NASA and USGS global mosaics

The following equirectangular surface maps are public-domain U.S. government works unless
otherwise noted. They are redistributed here as JPEG textures without logos or identifiers:

- `europa.jpg` — [Europa Voyager/Galileo SSI global mosaic](https://commons.wikimedia.org/wiki/File:Europa_Voyager_GalileoSSI_global_mosaic.jpg), U.S. Geological Survey / Planetary Data System / Tammy Becker; public domain.
- `callisto.jpg` — [Callisto Galileo/Voyager global map](https://commons.wikimedia.org/wiki/File:Callisto_USGS_global_small.jpg), USGS Astrogeology Science Center; public domain.
- `pluto.jpg` — [Pluto global color map](https://commons.wikimedia.org/wiki/File:Pluto_color_mapmosaic.jpg), NASA / Johns Hopkins University Applied Physics Laboratory / Southwest Research Institute; public domain.
- `ganymede.jpg` — [Map of Ganymede](https://commons.wikimedia.org/wiki/File:Map_of_Ganymede_by_Bj%C3%B6rn_J%C3%B3nsson.jpg), NASA source imagery assembled by Björn Jónsson. The author permits redistribution, modification, and commercial use with attribution; credit: Björn Jónsson.

## J2000 star background

`starfield-j2000-8k.jpg` is an sRGB JPEG conversion of the 8192 × 4096 OpenEXR map from
[NASA Goddard Scientific Visualization Studio, Deep Star Maps 2020](https://svs.gsfc.nasa.gov/4851/).
It plots the positions, brightness, and colors of stars from Hipparcos-2, Tycho-2, Gaia DR2,
the Yale Bright Star Catalog, UCAC3, and XHIP in ICRF/J2000 celestial coordinates. Credit:
NASA/Goddard Space Flight Center Scientific Visualization Studio; Gaia DR2: ESA/Gaia/DPAC.
The conversion applies display gamma and contrast only; star positions are unchanged.
