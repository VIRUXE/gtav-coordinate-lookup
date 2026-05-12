# GTAV Coordinate Lookup

Command-line tool for looking up GTA V zone and road information from `x y z` coordinates.

## What It Does

Given 3D GTA V map coordinates, it can return:

- Zone code, for example `AirP`.
- Zone name, for example `Los Santos International Airport`.
- Closest road.
- Nearby crossing road/intersection, when one is found.

Without `--output`, it returns the full result as JSON.

## Embedded Data

The GTAV data is embedded into the binary at build time:

- `data/zones.json`
- `data/nodes.zip`

This means:

- No data files need to be passed at runtime.
- The `data` folder does not need to sit next to the release binary.
- The files only need to exist when building.

The embedded dumps come from `DurtyFree/gta-v-data-dumps`.

## Build

Debug build:

```bash
cargo build
```

Release build:

```bash
cargo build --release
```

Final binary:

```bash
target/release/gtav-coordinate-lookup
```

## Basic Usage

Full JSON output, by default:

```bash
target/release/gtav-coordinate-lookup -1037.5 -2737.8 20.2
```

## Coordinate Input Formats

All of these are accepted:

```bash
target/release/gtav-coordinate-lookup -1037.5 -2737.8 20.2
target/release/gtav-coordinate-lookup "-1037.5,-2737.8,20.2"
target/release/gtav-coordinate-lookup -1037.5, -2737.8, 20.2
target/release/gtav-coordinate-lookup "vec(-1037.5, -2737.8, 20.2)"
target/release/gtav-coordinate-lookup "vec3(-1037.5, -2737.8, 20.2)"
target/release/gtav-coordinate-lookup "vec4(-1037.5, -2737.8, 20.2, 0.0)"
target/release/gtav-coordinate-lookup '{"x":-1037.5,"y":-2737.8,"z":20.2}'
target/release/gtav-coordinate-lookup '{"X":-1037.5,"Y":-2737.8,"Z":20.2}'
target/release/gtav-coordinate-lookup '[-1037.5,-2737.8,20.2]'
target/release/gtav-coordinate-lookup '[-1037.5,-2737.8,20.2,0.0]'
target/release/gtav-coordinate-lookup x=-1037.5 y=-2737.8 z=20.2
```

For `vec4` and JSON arrays with 4 values, the fourth value is accepted and ignored.

Using the release binary:

```bash
target/release/gtav-coordinate-lookup -1037.5 -2737.8 20.2
```

Expected output:

```json
{
  "coordinates": { "x": -1037.5, "y": -2737.8, "z": 20.2 },
  "zones": [
    { "name": "AirP", "displayName": "Los Santos International Airport", "match": "3d" }
  ],
  "road": { "name": "New Empire Way", "distance": 13.191523395015555, "from": "333:116", "to": "122" },
  "intersection": null
}
```

## Options

```text
-o, --output <fields>
```

Select specific fields to print.

Available output fields:

```text
zone-code
zone-name
road
intersection
json
all
```

`json` and `all` return the full JSON output.

```text
--street-radius <meters>
```

Only returns road/intersection results if they are within this distance in meters.

```text
-h, --help
```

Shows CLI help.

## Output Examples

Zone code:

```bash
target/release/gtav-coordinate-lookup --output zone-code -1037.5 -2737.8 20.2
```

```text
AirP
```

Zone name:

```bash
target/release/gtav-coordinate-lookup --output zone-name -1037.5 -2737.8 20.2
```

```text
Los Santos International Airport
```

Road:

```bash
target/release/gtav-coordinate-lookup --output road -1037.5 -2737.8 20.2
```

```text
New Empire Way
```

Intersection:

```bash
target/release/gtav-coordinate-lookup --output intersection 1850.0 3685.0 34.2
```

```text
Zancudo Ave
```

Multiple fields:

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection -1037.5 -2737.8 20.2
```

```json
{
  "zoneCode": "AirP",
  "zoneName": "Los Santos International Airport",
  "road": "New Empire Way",
  "intersection": null
}
```

## Street Radius

`--street-radius` helps avoid misleading results.

Without a radius, the tool always returns the nearest road, even if the coordinate is far away from any real road.

With a radius:

```bash
target/release/gtav-coordinate-lookup --street-radius 30 --output road -1037.5 -2737.8 20.2
```

If no road exists within `30` meters, the result is empty/null.

It also affects `intersection`, because the intersection is calculated as another nearby road around the same coordinate.

## Good Test Coordinates

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection 215.0 -810.0 30.7
```

Pillbox Hill, San Andreas Ave.

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection 1850.0 3685.0 34.2
```

Sandy Shores, Alhambra Dr / Zancudo Ave.

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection -134.0 6356.0 31.5
```

Paleto Bay, Pyrite Ave / Paleto Blvd.

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection 1700.0 4800.0 41.0
```

Grapeseed, Grapeseed Main St / Grapeseed Ave.

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection 425.0 -979.0 30.7
```

Mission Row, Atlee St / Sinner St.

```bash
target/release/gtav-coordinate-lookup --output zone-code,zone-name,road,intersection -48.0 -1757.0 29.4
```

Davis, Davis Ave / Grove St.

## How It Works

- Zones are matched using embedded 3D bounds.
- If no zone matches by `z`, the tool falls back to `x/y`.
- The road is calculated from the nearest road segment.
- The intersection is calculated as another nearby road different from the primary road.

## Limitations

- This is an offline lookup based on extracted data. It does not call game natives.
- Intersection lookup is a useful approximation, not a perfect simulation of GTA V internals.
- If the coordinate is far away from a road, use `--street-radius` to avoid false road results.
- Some zones may have an empty `displayName` in the source data. In those cases, specific outputs fall back to the zone code.

## Tests

```bash
cargo fmt
cargo test
```

Final build:

```bash
cargo build --release
```
