use std::{fs, io};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub output_port: u16,
    pub upstreams: Vec<String>,
    pub sitemap: Vec<SiteMapEntry>,
    pub max_memory_mb: u64,
}

#[derive(Deserialize)]
pub struct SiteMapEntry {
    pub loc: String,
    pub priority: f64,
}

static DEFAULT_PORT: u16 = 8080;
impl Config {
    pub fn new(args: Vec<String>) -> Config {
        if args.len() < 2 {
            panic!("Use a port of configuration file path as argument")
        }
        let mut proxy_port = DEFAULT_PORT;
        if args.len() == 3 && let Ok(proxy_port_arg) = args[2].parse::<u16>() {
                proxy_port = proxy_port_arg;
        }
        match args[1].parse::<u16>() {
            Ok(upstream_port) => Config::lite_mode(upstream_port, proxy_port),
            Err(_) => Config::read_file(&args[1]),
        }
    }

    fn lite_mode(upstream_port: u16, proxy_port: u16) -> Config {
        println!("Starting using lite mode on port {}", &proxy_port);
        Config {
            output_port: proxy_port,
            upstreams: vec![format!("http://localhost:{}", upstream_port)],
            sitemap: vec![],
            max_memory_mb: 32,
        }
    }

    fn read_file(path: &str) -> Config {
        println!("Starting using standard mode, reading from file");
        match path {
            "config.json" | "edgemap.json" => Config::read_json_config(path),
            "config.yaml" | "edgemap.yaml" => Config::read_yaml_config(path),
            "sitemap.xml" | "sitemap_index.xml" => Config::read_xml_sitemap(path),
            _ => panic!("Invalid file format or argument"),
        }
        .expect(&format!("Failed to parse file {}", path))
    }

    fn read_json_config(path: &str) -> Result<Config, io::Error> {
        let config_stringified = fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&config_stringified)?;
        Ok(config)
    }

    fn read_yaml_config(path: &str) -> Result<Config, io::Error> {
        todo!("Not yet implemented, use config.json or edgemap.json")
    }

    fn read_xml_sitemap(path: &str) -> Result<Config, io::Error> {
        todo!("Not yet implemented, use config.json or edgemap.json")
    }
}

#[cfg(test)]
mod tests {
    use reqwest::Method;

    use crate::cache::{CacheHandler, PathType, RequestData};

    use super::*;

    fn test_sitemap() -> Vec<SiteMapEntry> {
        vec![
            SiteMapEntry { loc: "/".to_string(), priority: 1.0 },
            SiteMapEntry { loc: "/public/*".to_string(), priority: 0.5 },
        ]
    }

    #[test]
    fn test_exact_match_allowed() {
        let cache = CacheHandler::new(test_sitemap(), 2);
        let req = RequestData {
            uri: "/".parse().unwrap(),
            method: Method::GET,
        };
        assert!(matches!(cache.check(&req), PathType::Public | PathType::Cached(_)));
    }

    #[test]
    fn test_wildcard_match_allowed() {
        let cache = CacheHandler::new(test_sitemap(), 2);
        let req = RequestData {
            uri: "/public/style.css".parse().unwrap(),
            method: Method::GET,
        };
        assert!(matches!(cache.check(&req), PathType::Public | PathType::Cached(_)));
    }

    #[test]
    fn test_api_path_blocked() {
        let cache = CacheHandler::new(test_sitemap(), 2);
        let req = RequestData {
            uri: "/api/users".parse().unwrap(),
            method: Method::GET,
        };
        assert!(matches!(cache.check(&req), PathType::Private));
    }
}
