use serde::Deserialize;
use std::cmp::Ordering;
use std::env;
use std::io::{Cursor, Read};
use std::process;
use zip::ZipArchive;

const ZONES_JSON: &str = include_str!("../data/zones.json");
const NODES_ZIP: &[u8] = include_bytes!("../data/nodes.zip");

type AppResult<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, Deserialize)]
struct Vec3 {
    #[serde(rename = "X")]
    x: f64,
    #[serde(rename = "Y")]
    y: f64,
    #[serde(rename = "Z")]
    z: f64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonCoordinates {
    Object(JsonCoordinateObject),
    Array(Vec<f64>),
}

#[derive(Debug, Deserialize)]
struct JsonCoordinateObject {
    #[serde(alias = "X")]
    x: f64,
    #[serde(alias = "Y")]
    y: f64,
    #[serde(alias = "Z")]
    z: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct Bounds {
    #[serde(rename = "Minimum")]
    min: Vec3,
    #[serde(rename = "Maximum")]
    max: Vec3,
}

#[derive(Clone, Debug, Deserialize)]
struct Zone {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "DisplayName")]
    display_name: Option<String>,
    #[serde(rename = "Bounds")]
    bounds: Vec<Bounds>,
}

#[derive(Clone, Debug, Deserialize)]
struct NodeCell {
    #[serde(rename = "AreaId")]
    area_id: u16,
    #[serde(rename = "Nodes")]
    nodes: Vec<Node>,
}

#[derive(Clone, Debug, Deserialize)]
struct Node {
    #[serde(rename = "Id")]
    id: u16,
    #[serde(rename = "StreetName")]
    street_name: String,
    #[serde(rename = "Position")]
    position: Vec3,
    #[serde(rename = "ConnectedNodes")]
    connected_nodes: Option<Vec<ConnectedNode>>,
}

#[derive(Clone, Debug, Deserialize)]
struct ConnectedNode {
    #[serde(rename = "Node")]
    node: Node,
}

#[derive(Clone, Debug)]
struct Config {
    street_radius: Option<f64>,
    output: OutputSelection,
    point: Vec3,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OutputSelection {
    AllJson,
    Fields(Vec<OutputField>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputField {
    ZoneCode,
    ZoneName,
    Road,
    Intersection,
}

#[derive(Clone, Debug)]
struct ZoneMatch<'a> {
    zone: &'a Zone,
    bounds: Bounds,
    match_type: MatchType,
}

#[derive(Clone, Copy, Debug)]
enum MatchType {
    Exact3d,
    XyFallback,
}

#[derive(Clone, Debug)]
struct RoadSegment {
    street_name: String,
    start_area_id: u16,
    start_node_id: u16,
    end_node_id: u16,
    start: Vec3,
    end: Vec3,
}

#[derive(Clone, Debug)]
struct RoadMatch<'a> {
    segment: &'a RoadSegment,
    distance: f64,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        eprintln!("run with --help for usage");
        process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let config = parse_args(env::args().skip(1).collect())?;
    let zones = load_zones()?;
    let road_segments = load_road_segments()?;

    let zone_matches = find_zones(&zones, config.point);
    let road_match = find_nearest_road(&road_segments, config.point, config.street_radius);
    let intersection_match = find_intersection_road(
        &road_segments,
        road_match.as_ref(),
        config.point,
        config.street_radius,
    );

    match config.output {
        OutputSelection::AllJson => {
            print_json(
                config.point,
                &zone_matches,
                road_match.as_ref(),
                intersection_match.as_ref(),
            );
        }
        OutputSelection::Fields(fields) => {
            print_selected_fields(
                &fields,
                &zone_matches,
                road_match.as_ref(),
                intersection_match.as_ref(),
            );
        }
    }

    Ok(())
}

fn parse_args(args: Vec<String>) -> AppResult<Config> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        process::exit(0);
    }

    let mut street_radius = None;
    let mut output = OutputSelection::AllJson;
    let mut coordinate_args = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--street-radius" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or("--street-radius needs a number")?
                    .parse::<f64>()
                    .map_err(|_| "--street-radius must be a valid number".to_string())?;

                if value < 0.0 {
                    return Err("--street-radius cannot be negative".to_string());
                }

                street_radius = Some(value);
            }
            "--output" | "-o" => {
                i += 1;
                output =
                    parse_output_selection(args.get(i).ok_or(
                        "--output needs zone-code, zone-name, road, intersection, or json",
                    )?)?;
            }
            arg if arg.starts_with('-')
                && arg.parse::<f64>().is_err()
                && !looks_like_coordinate_token(arg) =>
            {
                return Err(format!("unknown option: {arg}"));
            }
            arg => coordinate_args.push(arg.to_string()),
        }
        i += 1;
    }

    let point = parse_coordinates(&coordinate_args)?;

    Ok(Config {
        street_radius,
        output,
        point,
    })
}

fn looks_like_coordinate_token(value: &str) -> bool {
    let trimmed = value.trim_start_matches(['(', '[']);
    let Some(after_minus) = trimmed.strip_prefix('-') else {
        return false;
    };

    after_minus
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit() || character == '.')
}

fn parse_coordinates(args: &[String]) -> AppResult<Vec3> {
    if args.is_empty() {
        return Err("expected coordinates: x y z".to_string());
    }

    let input = args.join(" ");

    if let Some(result) = parse_json_coordinates(&input) {
        return result;
    }

    if let Some(result) = parse_key_value_coordinates(&input) {
        return result;
    }

    parse_numeric_coordinates(&input)
}

fn parse_json_coordinates(input: &str) -> Option<AppResult<Vec3>> {
    let trimmed = input.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }

    let parsed = match serde_json::from_str::<JsonCoordinates>(trimmed) {
        Ok(parsed) => parsed,
        Err(error) => return Some(Err(format!("invalid JSON coordinates: {error}"))),
    };

    Some(match parsed {
        JsonCoordinates::Object(object) => Ok(Vec3 {
            x: object.x,
            y: object.y,
            z: object.z,
        }),
        JsonCoordinates::Array(values) if values.len() >= 3 => Ok(Vec3 {
            x: values[0],
            y: values[1],
            z: values[2],
        }),
        JsonCoordinates::Array(_) => {
            Err("JSON coordinate arrays need at least 3 numbers".to_string())
        }
    })
}

fn parse_key_value_coordinates(input: &str) -> Option<AppResult<Vec3>> {
    if !input.contains('=') {
        return None;
    }

    let cleaned = input.replace([',', '(', ')'], " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    let mut x = None;
    let mut y = None;
    let mut z = None;
    let mut index = 0;

    while index < tokens.len() {
        let token = tokens[index].trim();
        let lower = token.to_ascii_lowercase();

        if let Some((key, _value)) = lower.split_once('=') {
            let original_value = token
                .split_once('=')
                .map(|(_, value)| value)
                .unwrap_or_default();
            let value = if original_value.is_empty() {
                index += 1;
                tokens.get(index).copied().unwrap_or_default()
            } else {
                original_value
            };

            if let Err(error) = set_coordinate_value(key, value, &mut x, &mut y, &mut z) {
                return Some(Err(error));
            }
        } else if matches!(lower.as_str(), "x" | "y" | "z")
            && tokens.get(index + 1).is_some_and(|value| *value == "=")
        {
            let value = tokens.get(index + 2).copied().unwrap_or_default();
            if let Err(error) = set_coordinate_value(&lower, value, &mut x, &mut y, &mut z) {
                return Some(Err(error));
            }
            index += 2;
        }

        index += 1;
    }

    Some(match (x, y, z) {
        (Some(x), Some(y), Some(z)) => Ok(Vec3 { x, y, z }),
        _ => Err("expected x=, y= and z= coordinates".to_string()),
    })
}

fn set_coordinate_value(
    key: &str,
    value: &str,
    x: &mut Option<f64>,
    y: &mut Option<f64>,
    z: &mut Option<f64>,
) -> AppResult<()> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| format!("invalid {key} coordinate: {value}"))?;

    match key {
        "x" => *x = Some(parsed),
        "y" => *y = Some(parsed),
        "z" => *z = Some(parsed),
        _ => {}
    }

    Ok(())
}

fn parse_numeric_coordinates(input: &str) -> AppResult<Vec3> {
    let (content, is_vector) = strip_vector_wrapper(input);
    let numbers = extract_numbers(content);

    if numbers.len() == 3 || (is_vector && numbers.len() >= 3) {
        return Ok(Vec3 {
            x: numbers[0],
            y: numbers[1],
            z: numbers[2],
        });
    }

    Err("expected coordinates as x y z, x,y,z, vec(...), vec3(...), vec4(...), x=... y=... z=..., {\"x\":...}, or [x,y,z]".to_string())
}

fn strip_vector_wrapper(input: &str) -> (&str, bool) {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();

    for prefix in ["vector4", "vector3", "vec4", "vec3", "vec"] {
        if !lower.starts_with(prefix) {
            continue;
        }

        let after_prefix = &trimmed[prefix.len()..];
        let content = after_prefix
            .trim()
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .unwrap_or(after_prefix);

        return (content, true);
    }

    (trimmed, false)
}

fn extract_numbers(input: &str) -> Vec<f64> {
    input
        .chars()
        .map(|character| {
            if character.is_ascii_digit() || matches!(character, '.' | '-' | '+' | 'e' | 'E') {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter_map(|part| part.parse::<f64>().ok())
        .collect()
}

fn parse_output_selection(value: &str) -> AppResult<OutputSelection> {
    if matches!(value, "json" | "all") {
        return Ok(OutputSelection::AllJson);
    }

    let mut fields = Vec::new();
    for part in value.split(',') {
        let field = match part.trim() {
            "zone-code" => OutputField::ZoneCode,
            "zone-name" => OutputField::ZoneName,
            "road" => OutputField::Road,
            "intersection" => OutputField::Intersection,
            "" => return Err("empty output field".to_string()),
            invalid => {
                return Err(format!(
                    "invalid output field: {invalid}. Use zone-code, zone-name, road, intersection, or json"
                ));
            }
        };

        if !fields.contains(&field) {
            fields.push(field);
        }
    }

    if fields.is_empty() {
        return Err("--output needs at least one field".to_string());
    }

    Ok(OutputSelection::Fields(fields))
}

fn print_help() {
    println!(
        "GTAV coordinate lookup\n\
         \n\
         Usage:\n\
          gtav-coordinate-lookup <x> <y> <z>\n\
          gtav-coordinate-lookup \"vec3(<x>, <y>, <z>)\"\n\
          gtav-coordinate-lookup '{{\"x\":<x>,\"y\":<y>,\"z\":<z>}}'\n\
          gtav-coordinate-lookup x=<x> y=<y> z=<z>\n\
         \n\
         Options:\n\
           --street-radius <meters>   Ignore roads further than this distance\n\
          -o, --output <fields>      Fields: zone-code, zone-name, road, intersection, json\n\
           -h, --help                 Show this help\n\
         \n\
         Example:\n\
          gtav-coordinate-lookup -1037.5 -2737.8 20.2\n\
          gtav-coordinate-lookup --output road,intersection -1037.5 -2737.8 20.2"
    );
}

fn load_zones() -> AppResult<Vec<Zone>> {
    serde_json::from_str(ZONES_JSON)
        .map_err(|error| format!("failed to parse embedded zones: {error}"))
}

fn load_road_segments() -> AppResult<Vec<RoadSegment>> {
    let cursor = Cursor::new(NODES_ZIP);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|error| format!("failed to open embedded nodes: {error}"))?;
    let mut nodes_json = String::new();

    archive
        .by_name("nodes.json")
        .map_err(|error| format!("failed to read embedded nodes.json: {error}"))?
        .read_to_string(&mut nodes_json)
        .map_err(|error| format!("failed to decode embedded nodes.json: {error}"))?;

    let cells: Vec<NodeCell> = serde_json::from_str(&nodes_json)
        .map_err(|error| format!("failed to parse embedded road nodes: {error}"))?;

    let mut segments = Vec::new();
    for cell in cells {
        for node in cell.nodes {
            let Some(connected_nodes) = node.connected_nodes.as_ref() else {
                continue;
            };

            for connected in connected_nodes {
                let street_name = pick_street_name(&node.street_name, &connected.node.street_name);
                if street_name.is_empty() {
                    continue;
                }

                segments.push(RoadSegment {
                    street_name: street_name.to_string(),
                    start_area_id: cell.area_id,
                    start_node_id: node.id,
                    end_node_id: connected.node.id,
                    start: node.position,
                    end: connected.node.position,
                });
            }
        }
    }

    if segments.is_empty() {
        return Err("embedded road node data did not contain any named road segments".to_string());
    }

    Ok(segments)
}

fn pick_street_name<'a>(first: &'a str, second: &'a str) -> &'a str {
    if is_real_street_name(first) {
        first
    } else if is_real_street_name(second) {
        second
    } else {
        ""
    }
}

fn is_real_street_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed != "0"
}

fn find_zones<'a>(zones: &'a [Zone], point: Vec3) -> Vec<ZoneMatch<'a>> {
    let mut matches = find_zones_by(zones, point, contains_3d, MatchType::Exact3d);

    if matches.is_empty() {
        matches = find_zones_by(zones, point, contains_xy, MatchType::XyFallback);
    }

    matches.sort_by(|a, b| {
        bounds_volume(a.bounds)
            .partial_cmp(&bounds_volume(b.bounds))
            .unwrap_or(Ordering::Equal)
    });
    matches
}

fn find_zones_by<'a>(
    zones: &'a [Zone],
    point: Vec3,
    predicate: fn(Bounds, Vec3) -> bool,
    match_type: MatchType,
) -> Vec<ZoneMatch<'a>> {
    zones
        .iter()
        .flat_map(|zone| {
            zone.bounds
                .iter()
                .copied()
                .filter(move |bounds| predicate(*bounds, point))
                .map(move |bounds| ZoneMatch {
                    zone,
                    bounds,
                    match_type,
                })
        })
        .collect()
}

fn contains_3d(bounds: Bounds, point: Vec3) -> bool {
    contains_xy(bounds, point) && point.z >= bounds.min.z && point.z <= bounds.max.z
}

fn contains_xy(bounds: Bounds, point: Vec3) -> bool {
    point.x >= bounds.min.x
        && point.x <= bounds.max.x
        && point.y >= bounds.min.y
        && point.y <= bounds.max.y
}

fn bounds_volume(bounds: Bounds) -> f64 {
    (bounds.max.x - bounds.min.x).abs()
        * (bounds.max.y - bounds.min.y).abs()
        * (bounds.max.z - bounds.min.z).abs().max(1.0)
}

fn find_nearest_road(
    segments: &[RoadSegment],
    point: Vec3,
    street_radius: Option<f64>,
) -> Option<RoadMatch<'_>> {
    segments
        .iter()
        .filter_map(|segment| {
            let distance = distance_to_segment(point, segment.start, segment.end);

            if street_radius.is_some_and(|radius| distance > radius) {
                return None;
            }

            Some(RoadMatch { segment, distance })
        })
        .min_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(Ordering::Equal)
        })
}

fn find_intersection_road<'a>(
    segments: &'a [RoadSegment],
    road_match: Option<&RoadMatch<'a>>,
    point: Vec3,
    street_radius: Option<f64>,
) -> Option<RoadMatch<'a>> {
    let primary = road_match?;
    let max_distance = street_radius.unwrap_or(35.0).max(primary.distance);

    segments
        .iter()
        .filter(|segment| segment.street_name != primary.segment.street_name)
        .filter_map(|segment| {
            let distance = distance_to_segment(point, segment.start, segment.end);

            if distance > max_distance {
                return None;
            }

            Some(RoadMatch { segment, distance })
        })
        .min_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(Ordering::Equal)
        })
}

fn distance_to_segment(point: Vec3, start: Vec3, end: Vec3) -> f64 {
    let ab = Vec3 {
        x: end.x - start.x,
        y: end.y - start.y,
        z: end.z - start.z,
    };
    let ap = Vec3 {
        x: point.x - start.x,
        y: point.y - start.y,
        z: point.z - start.z,
    };
    let ab_len_sq = dot(ab, ab);

    if ab_len_sq == 0.0 {
        return distance(point, start);
    }

    let t = (dot(ap, ab) / ab_len_sq).clamp(0.0, 1.0);
    let closest = Vec3 {
        x: start.x + ab.x * t,
        y: start.y + ab.y * t,
        z: start.z + ab.z * t,
    };
    distance(point, closest)
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn print_json(
    point: Vec3,
    zone_matches: &[ZoneMatch<'_>],
    road_match: Option<&RoadMatch<'_>>,
    intersection_match: Option<&RoadMatch<'_>>,
) {
    println!("{{");
    println!(
        "  \"coordinates\": {{ \"x\": {}, \"y\": {}, \"z\": {} }},",
        json_number(point.x),
        json_number(point.y),
        json_number(point.z)
    );
    println!("  \"zones\": [");
    for (index, zone_match) in zone_matches.iter().enumerate() {
        let comma = if index + 1 == zone_matches.len() {
            ""
        } else {
            ","
        };
        let match_type = match zone_match.match_type {
            MatchType::Exact3d => "3d",
            MatchType::XyFallback => "xy_fallback",
        };
        println!(
            "    {{ \"name\": \"{}\", \"displayName\": {}, \"match\": \"{}\" }}{}",
            json_escape(&zone_match.zone.name),
            json_string_or_null(zone_match.zone.display_name.as_deref()),
            match_type,
            comma
        );
    }
    println!("  ],");

    match road_match {
        Some(road) => println!(
            "  \"road\": {{ \"name\": \"{}\", \"distance\": {}, \"from\": \"{}:{}\", \"to\": \"{}\" }},",
            json_escape(&road.segment.street_name),
            json_number(road.distance),
            road.segment.start_area_id,
            road.segment.start_node_id,
            road.segment.end_node_id
        ),
        None => println!("  \"road\": null,"),
    }

    match intersection_match {
        Some(road) => println!(
            "  \"intersection\": {{ \"name\": \"{}\", \"distance\": {}, \"from\": \"{}:{}\", \"to\": \"{}\" }}",
            json_escape(&road.segment.street_name),
            json_number(road.distance),
            road.segment.start_area_id,
            road.segment.start_node_id,
            road.segment.end_node_id
        ),
        None => println!("  \"intersection\": null"),
    }
    println!("}}");
}

fn print_selected_fields(
    fields: &[OutputField],
    zone_matches: &[ZoneMatch<'_>],
    road_match: Option<&RoadMatch<'_>>,
    intersection_match: Option<&RoadMatch<'_>>,
) {
    if fields.len() == 1 {
        println!(
            "{}",
            selected_field_value(fields[0], zone_matches, road_match, intersection_match)
                .unwrap_or_default()
        );
        return;
    }

    println!("{{");
    for (index, field) in fields.iter().enumerate() {
        let comma = if index + 1 == fields.len() { "" } else { "," };
        let value = selected_field_value(*field, zone_matches, road_match, intersection_match);

        println!(
            "  \"{}\": {}{}",
            output_field_key(*field),
            json_string_or_null(value.as_deref()),
            comma
        );
    }
    println!("}}");
}

fn selected_field_value(
    field: OutputField,
    zone_matches: &[ZoneMatch<'_>],
    road_match: Option<&RoadMatch<'_>>,
    intersection_match: Option<&RoadMatch<'_>>,
) -> Option<String> {
    match field {
        OutputField::ZoneCode => zone_matches.first().map(|zone| zone.zone.name.clone()),
        OutputField::ZoneName => zone_matches.first().map(|zone| {
            zone.zone
                .display_name
                .clone()
                .unwrap_or_else(|| zone.zone.name.clone())
        }),
        OutputField::Road => road_match.map(|road| road.segment.street_name.clone()),
        OutputField::Intersection => {
            intersection_match.map(|road| road.segment.street_name.clone())
        }
    }
}

fn output_field_key(field: OutputField) -> &'static str {
    match field {
        OutputField::ZoneCode => "zoneCode",
        OutputField::ZoneName => "zoneName",
        OutputField::Road => "road",
        OutputField::Intersection => "intersection",
    }
}

fn json_string_or_null(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", json_escape(value)))
        .unwrap_or_else(|| "null".to_string())
}

fn json_number(value: f64) -> String {
    if value.is_finite() {
        value.to_string()
    } else {
        "null".to_string()
    }
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_zones_are_available() {
        let zones = load_zones().unwrap();

        assert!(zones.len() >= 90);
        assert!(zones
            .iter()
            .any(|zone| zone.name.eq_ignore_ascii_case("AirP")));
    }

    #[test]
    fn embedded_roads_are_available() {
        let roads = load_road_segments().unwrap();

        assert!(roads.len() > 10_000);
        assert!(roads.iter().any(|road| road.street_name == "Ineseno Road"));
    }

    #[test]
    fn finds_airport_zone_from_embedded_data() {
        let zones = load_zones().unwrap();
        let matches = find_zones(
            &zones,
            Vec3 {
                x: -1037.5,
                y: -2737.8,
                z: 20.2,
            },
        );

        assert!(matches
            .iter()
            .any(|zone| zone.zone.name.eq_ignore_ascii_case("AirP")));
    }

    #[test]
    fn defaults_to_full_json_output() {
        let config = parse_args(vec![
            "-1037.5".to_string(),
            "-2737.8".to_string(),
            "20.2".to_string(),
        ])
        .unwrap();

        assert_eq!(config.output, OutputSelection::AllJson);
    }

    #[test]
    fn parses_output_field_parameter() {
        let config = parse_args(vec![
            "--output".to_string(),
            "road,intersection".to_string(),
            "-1037.5".to_string(),
            "-2737.8".to_string(),
            "20.2".to_string(),
        ])
        .unwrap();

        assert_eq!(
            config.output,
            OutputSelection::Fields(vec![OutputField::Road, OutputField::Intersection])
        );
    }

    #[test]
    fn parses_comma_separated_coordinates() {
        let point = parse_coordinates(&["-1037.5,-2737.8,20.2".to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_spaced_comma_coordinates() {
        let point = parse_coordinates(&[
            "-1037.5,".to_string(),
            "-2737.8,".to_string(),
            "20.2".to_string(),
        ])
        .unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_vector_coordinates() {
        let point = parse_coordinates(&["vec3(-1037.5, -2737.8, 20.2)".to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_vec4_coordinates_and_ignores_w() {
        let point =
            parse_coordinates(&["vec4(-1037.5, -2737.8, 20.2, 999.0)".to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_vec_coordinates_with_w_and_ignores_w() {
        let point = parse_coordinates(&["vec(-1037.5, -2737.8, 20.2, 999.0)".to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_key_value_coordinates() {
        let point = parse_coordinates(&[
            "x=-1037.5".to_string(),
            "y=-2737.8".to_string(),
            "z=20.2".to_string(),
        ])
        .unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_json_object_coordinates() {
        let point =
            parse_coordinates(&[r#"{"x":-1037.5,"y":-2737.8,"z":20.2}"#.to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_json_object_coordinates_with_uppercase_keys() {
        let point =
            parse_coordinates(&[r#"{"X":-1037.5,"Y":-2737.8,"Z":20.2}"#.to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_json_array_coordinates() {
        let point = parse_coordinates(&["[-1037.5,-2737.8,20.2]".to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }

    #[test]
    fn parses_json_array_coordinates_and_ignores_w() {
        let point = parse_coordinates(&["[-1037.5,-2737.8,20.2,0.0]".to_string()]).unwrap();

        assert_eq!(point.x, -1037.5);
        assert_eq!(point.y, -2737.8);
        assert_eq!(point.z, 20.2);
    }
}
