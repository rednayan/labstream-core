//! The configuration file.
//!
//! liblsl reads one file at the first call that needs it, keeps the result for
//! the life of the process, and never reads another
//! (`src/api_config.cpp:326-337`). Every port, every multicast group, and
//! every tuning value below comes from that file or from a default.
//!
//! A site that changes `lsl_api.cfg` changes the wire. Two libraries that read
//! the file differently cannot see each other, and no test of the protocol
//! finds that, because both sides are correct on their own defaults.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// A file in the format that liblsl reads.
///
/// The rules come from `src/util/inireader.cpp`:
///
/// * A line that starts with `;` is a comment. **A `#` is not.**
/// * A blank line is skipped.
/// * `[section]` opens a section. The key is then `section.Key`.
/// * A line with no `=` is an error, and one error throws the whole file away.
/// * A repeated key is an error.
/// * The key and the value lose the space around them.
#[derive(Debug, Default, Clone)]
pub struct Ini {
    values: HashMap<String, String>,
}

impl Ini {
    /// Read a file. An error names the line.
    pub fn parse(text: &str) -> Result<Ini, String> {
        let mut values: HashMap<String, String> = HashMap::new();
        let mut section = String::new();
        for (k, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            let number = k + 1;
            if line.starts_with(';') || line.trim().is_empty() {
                continue;
            }
            if line.starts_with('[') {
                let close = line
                    .find(']')
                    .ok_or_else(|| format!("No closing bracket ] found in line {number}"))?;
                section = format!("{}.", &line[1..close]);
                continue;
            }
            let eq = line
                .find('=')
                .ok_or_else(|| format!("No Key-Value pair in line {number}"))?;
            let key = line[..eq].trim();
            let value = line[eq + 1..].trim();
            if key.is_empty() {
                return Err(format!("Empty key in line {number}"));
            }
            if value.is_empty() {
                return Err(format!("Empty value in line {number}"));
            }
            let full = format!("{section}{key}");
            if values.contains_key(&full) {
                return Err(format!("Duplicate key {full}"));
            }
            values.insert(full, value.to_string());
        }
        Ok(Ini { values })
    }

    /// The text of a key, or a default.
    pub fn text(&self, key: &str, default: &str) -> String {
        self.values
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    /// A whole number, read the way `std::istream` reads one.
    pub fn int(&self, key: &str, default: i64) -> i64 {
        match self.values.get(key) {
            Some(v) => leading_number(v).unwrap_or(0.0) as i64,
            None => default,
        }
    }

    /// A number with a fraction.
    pub fn float(&self, key: &str, default: f64) -> f64 {
        match self.values.get(key) {
            Some(v) => leading_number(v).unwrap_or(0.0),
            None => default,
        }
    }

    /// A yes or no value.
    ///
    /// **Only the text `1` means yes.** `src/util/inireader.hpp` reads a
    /// boolean with `from_string<bool>`, and `src/util/cast.hpp:10` defines
    /// that as `str == "1"`. A file that says `true` therefore means no.
    pub fn flag(&self, key: &str, default: bool) -> bool {
        match self.values.get(key) {
            Some(v) => v == "1",
            None => default,
        }
    }

    /// A set of the form `{a, b, c}`.
    ///
    /// Anything that does not start with `{` and end with `}` is an empty set
    /// (`src/api_config.cpp:40-46`).
    pub fn set(&self, key: &str, default: &str) -> Vec<String> {
        let text = self.text(key, default);
        let t = text.trim();
        if t.len() <= 2 || !t.starts_with('{') || !t.ends_with('}') {
            return Vec::new();
        }
        t[1..t.len() - 1]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Read the longest leading run that looks like a number.
fn leading_number(text: &str) -> Option<f64> {
    let t = text.trim();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        let keep = c.is_ascii_digit()
            || (i == 0 && (c == '-' || c == '+'))
            || c == '.'
            || c == 'e'
            || c == 'E';
        if !keep {
            break;
        }
        end = i + c.len_utf8();
    }
    t[..end].parse().ok()
}

/// Every value that changes what travels on the wire.
///
/// The names follow `src/api_config.cpp:168-322`. A value that this
/// implementation does not act on yet is left out rather than read and
/// ignored.
#[derive(Debug, Clone)]
pub struct Config {
    /// The port for a multicast or broadcast query. `ports.MulticastPort`.
    pub multicast_port: u16,
    /// The first port of the unicast range. `ports.BasePort`.
    pub base_port: u16,
    /// How many ports the range holds. `ports.PortRange`.
    pub port_range: u16,
    /// The session that a stream belongs to. `lab.SessionID`.
    pub session_id: String,
    /// Addresses that a resolver always sends to. `lab.KnownPeers`.
    pub known_peers: Vec<String>,
    /// The groups that a query goes to, for the chosen scope.
    pub multicast_addresses: Vec<String>,
    /// The hop count for a multicast query.
    pub multicast_ttl: u32,
    /// How long a quiet source stays connected. `tuning.WatchdogTimeThreshold`.
    pub watchdog_time_threshold: f64,
    /// How often the clock is measured. `tuning.TimeUpdateInterval`.
    pub time_update_interval: f64,
    /// How many probes an estimate needs. `tuning.TimeUpdateMinProbes`.
    pub time_update_min_probes: u32,
    /// How many probes one burst sends. `tuning.TimeProbeCount`.
    pub time_probe_count: u32,
    /// The gap between two probes. `tuning.TimeProbeInterval`.
    pub time_probe_interval: f64,
    /// How long a burst waits for answers. `tuning.TimeProbeMaxRTT`.
    pub time_probe_max_rtt: f64,
    /// The half-time of the smoothing filter. `tuning.SmoothingHalftime`.
    pub smoothing_halftime: f32,
    /// When true, every pushed timestamp is replaced by the current clock.
    /// `tuning.ForceDefaultTimestamps`.
    pub force_default_timestamps: bool,
    /// The gap between two waves of a background resolver.
    /// `tuning.ContinuousResolveInterval`.
    pub continuous_resolve_interval: f64,
    /// The shortest time a multicast wave waits. `tuning.MulticastMinRTT`.
    pub multicast_min_rtt: f64,
    /// The shortest time a unicast wave waits. `tuning.UnicastMinRTT`.
    pub unicast_min_rtt: f64,
    /// The protocol version that an outlet writes. `tuning.UseProtocolVersion`.
    pub use_protocol_version: i32,
    /// True when an outlet binds an IPv4 stack. `ports.IPv6`.
    pub allow_ipv4: bool,
    /// True when an outlet binds an IPv6 stack. `ports.IPv6`.
    pub allow_ipv6: bool,
    /// Where the values came from, for a report.
    pub source: String,
}

impl Default for Config {
    fn default() -> Config {
        Config::from_ini(&Ini::default(), "the defaults")
    }
}

impl Config {
    /// Read a configuration out of a file that was already parsed.
    pub fn from_ini(ini: &Ini, source: &str) -> Config {
        // The scope decides which groups a query reaches, and the hop count.
        // `src/api_config.cpp:211-247`.
        let scope = ini.text("multicast.ResolveScope", "site");
        let rank = |s: &str| match s {
            "machine" => 0,
            "link" => 1,
            "site" => 2,
            "organization" => 3,
            "global" => 4,
            // liblsl throws here, which throws the whole file away.
            _ => 2,
        };
        let level = rank(&scope);
        // Each scope adds an IPv6 group as well, built from one suffix and the
        // prefix of that scope (`src/api_config.cpp:191-247`).
        let v6 = ini.text(
            "multicast.IPv6MulticastGroup",
            "113D:6FDD:2C17:A643:FFE2:1BD1:3CD2",
        );
        let mut addresses = ini.set("multicast.MachineAddresses", "{127.0.0.1}");
        let mut ttl = 0;
        if level >= 1 {
            addresses.extend(ini.set(
                "multicast.LinkAddresses",
                "{255.255.255.255, 224.0.0.1, 224.0.0.183}",
            ));
            addresses.push(format!("FF02:{v6}"));
            ttl = 1;
        }
        if level >= 2 {
            addresses.extend(ini.set("multicast.SiteAddresses", "{239.255.172.215}"));
            addresses.push(format!("FF05:{v6}"));
            ttl = 24;
        }
        if level >= 3 {
            addresses.extend(ini.set("multicast.OrganizationAddresses", "{}"));
            addresses.push(format!("FF08:{v6}"));
            ttl = 32;
        }
        if level >= 4 {
            addresses.extend(ini.set("multicast.GlobalAddresses", "{}"));
            addresses.push(format!("FF0E:{v6}"));
            ttl = 255;
        }
        let ttl_override = ini.int("multicast.TTLOverride", -1);
        if ttl_override >= 0 {
            ttl = ttl_override as u32;
        }
        let address_override = ini.set("multicast.AddressesOverride", "{}");
        if !address_override.is_empty() {
            addresses = address_override;
        }
        // `ports.IPv6` names which stacks an outlet binds
        // (`src/api_config.cpp:172-185`). A word outside the list makes liblsl
        // throw, which throws the whole file away, so the defaults apply.
        let (allow_ipv4, allow_ipv6) = match ini.text("ports.IPv6", "allow").as_str() {
            "disabled" | "disable" => (true, false),
            "forced" | "force" => (false, true),
            _ => (true, true),
        };
        addresses.retain(|a| match a.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(_)) => allow_ipv4,
            Ok(std::net::IpAddr::V6(_)) => allow_ipv6,
            Err(_) => false,
        });

        Config {
            multicast_port: ini.int("ports.MulticastPort", 16571) as u16,
            base_port: ini.int("ports.BasePort", 16572) as u16,
            port_range: ini.int("ports.PortRange", 32) as u16,
            session_id: ini.text("lab.SessionID", "default"),
            known_peers: ini.set("lab.KnownPeers", "{}"),
            multicast_addresses: addresses,
            multicast_ttl: ttl,
            watchdog_time_threshold: ini.float("tuning.WatchdogTimeThreshold", 15.0),
            time_update_interval: ini.float("tuning.TimeUpdateInterval", 2.0),
            time_update_min_probes: ini.int("tuning.TimeUpdateMinProbes", 6) as u32,
            time_probe_count: ini.int("tuning.TimeProbeCount", 8) as u32,
            time_probe_interval: ini.float("tuning.TimeProbeInterval", 0.064),
            time_probe_max_rtt: ini.float("tuning.TimeProbeMaxRTT", 0.128),
            smoothing_halftime: ini.float("tuning.SmoothingHalftime", 90.0) as f32,
            force_default_timestamps: ini.flag("tuning.ForceDefaultTimestamps", false),
            continuous_resolve_interval: ini.float("tuning.ContinuousResolveInterval", 0.5),
            multicast_min_rtt: ini.float("tuning.MulticastMinRTT", 0.5),
            unicast_min_rtt: ini.float("tuning.UnicastMinRTT", 0.75),
            use_protocol_version: (ini.int("tuning.UseProtocolVersion", 110) as i32).min(110),
            allow_ipv4,
            allow_ipv6,
            source: source.to_string(),
        }
    }
}

/// A file name that an application set before the first use.
static FILENAME: Mutex<Option<String>> = Mutex::new(None);
/// Text that an application set before the first use.
static CONTENT: Mutex<Option<String>> = Mutex::new(None);
static LOADED: OnceLock<Config> = OnceLock::new();

/// Name the file to read. Call this before anything else.
///
/// `lsl_set_config_filename`. A call after the configuration is read has no
/// effect, on either implementation.
pub fn set_filename(path: &str) {
    *FILENAME.lock().unwrap() = Some(path.to_string());
}

/// Give the configuration as text. Call this before anything else.
///
/// `lsl_set_config_content`. liblsl uses the text once and then drops it
/// (`src/api_config.cpp:57-70`).
pub fn set_content(text: &str) {
    *CONTENT.lock().unwrap() = Some(text.to_string());
}

/// The configuration of this process.
///
/// The file is read once. Every later call returns the same values, which is
/// what liblsl does with `std::call_once` (`src/api_config.cpp:326`).
pub fn get() -> &'static Config {
    LOADED.get_or_init(load)
}

/// Where liblsl looks, in order. `src/api_config.cpp:70-110`.
fn candidates() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(name) = FILENAME.lock().unwrap().clone() {
        if std::fs::File::open(&name).is_ok() {
            out.push(name);
        }
    }
    // The environment takes precedence over the name that an application set.
    if let Ok(env) = std::env::var("LSLAPICFG") {
        if std::fs::File::open(&env).is_ok() {
            out.insert(0, env);
        }
    }
    out.push("lsl_api.cfg".to_string());
    if let Ok(home) = std::env::var("HOME") {
        out.push(format!("{home}/lsl_api/lsl_api.cfg"));
    }
    out.push("/etc/lsl_api/lsl_api.cfg".to_string());
    out
}

fn load() -> Config {
    if let Some(text) = CONTENT.lock().unwrap().take() {
        match Ini::parse(&text) {
            Ok(ini) => return Config::from_ini(&ini, "the text that the application gave"),
            // A file that does not parse leaves the defaults in place.
            Err(_) => return Config::default(),
        }
    }
    for name in candidates() {
        if let Ok(text) = std::fs::read_to_string(&name) {
            return match Ini::parse(&text) {
                Ok(ini) => Config::from_ini(&ini, &name),
                Err(_) => Config::default(),
            };
        }
    }
    Config::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_becomes_part_of_the_key() {
        let ini = Ini::parse("[ports]\nBasePort = 17000\n").expect("a file");
        assert_eq!(ini.int("ports.BasePort", 16572), 17000);
        assert_eq!(ini.int("BasePort", 16572), 16572);
    }

    #[test]
    fn only_a_semicolon_starts_a_comment() {
        // `src/util/inireader.cpp:13` tests for `;` and nothing else. A `#`
        // makes the line a key with no `=`, which throws the file away.
        assert!(Ini::parse("; a note\n[lab]\nSessionID = x\n").is_ok());
        assert!(Ini::parse("# a note\n[lab]\nSessionID = x\n").is_err());
    }

    #[test]
    fn only_the_digit_one_means_yes() {
        // `src/util/cast.hpp:10` defines this as `str == "1"`.
        let ini = Ini::parse("[tuning]\nA = 1\nB = true\nC = 0\n").expect("a file");
        assert!(ini.flag("tuning.A", false));
        assert!(!ini.flag("tuning.B", false));
        assert!(!ini.flag("tuning.C", true));
        assert!(ini.flag("tuning.Missing", true));
    }

    #[test]
    fn a_repeated_key_throws_the_file_away() {
        assert!(Ini::parse("[lab]\nSessionID = a\nSessionID = b\n").is_err());
    }

    #[test]
    fn a_line_with_no_equals_sign_throws_the_file_away() {
        assert!(Ini::parse("[lab]\nSessionID\n").is_err());
    }

    #[test]
    fn a_set_needs_both_braces() {
        let ini = Ini::parse("[lab]\nA = {x, y}\nB = x, y\nC = {}\n").expect("a file");
        assert_eq!(ini.set("lab.A", "{}"), vec!["x", "y"]);
        assert!(ini.set("lab.B", "{}").is_empty());
        assert!(ini.set("lab.C", "{}").is_empty());
    }

    #[test]
    fn the_space_around_a_key_and_a_value_is_dropped() {
        let ini = Ini::parse("[lab]\n   SessionID   =   my lab   \n").expect("a file");
        assert_eq!(ini.text("lab.SessionID", ""), "my lab");
    }

    #[test]
    fn the_defaults_match_the_values_in_the_source() {
        let c = Config::default();
        assert_eq!(c.multicast_port, 16571);
        assert_eq!(c.base_port, 16572);
        assert_eq!(c.port_range, 32);
        assert_eq!(c.session_id, "default");
        assert_eq!(c.watchdog_time_threshold, 15.0);
        assert_eq!(c.time_update_interval, 2.0);
        assert_eq!(c.time_update_min_probes, 6);
        assert_eq!(c.time_probe_count, 8);
        assert_eq!(c.time_probe_interval, 0.064);
        assert_eq!(c.time_probe_max_rtt, 0.128);
        assert_eq!(c.smoothing_halftime, 90.0);
        assert!(!c.force_default_timestamps);
        assert_eq!(c.use_protocol_version, 110);
    }

    #[test]
    fn the_scope_decides_the_groups_and_the_hop_count() {
        let of = |scope: &str| {
            let ini =
                Ini::parse(&format!("[multicast]\nResolveScope = {scope}\n")).expect("a file");
            Config::from_ini(&ini, "test")
        };
        let machine = of("machine");
        assert_eq!(machine.multicast_addresses, vec!["127.0.0.1"]);
        assert_eq!(machine.multicast_ttl, 0);

        let link = of("link");
        assert!(link
            .multicast_addresses
            .contains(&"224.0.0.183".to_string()));
        assert!(!link
            .multicast_addresses
            .contains(&"239.255.172.215".to_string()));
        assert_eq!(link.multicast_ttl, 1);

        let site = of("site");
        assert!(site
            .multicast_addresses
            .contains(&"239.255.172.215".to_string()));
        assert_eq!(site.multicast_ttl, 24);
    }

    #[test]
    fn an_address_override_replaces_the_whole_list() {
        let ini =
            Ini::parse("[multicast]\nAddressesOverride = {10.0.0.1, 10.0.0.2}\n").expect("a file");
        let c = Config::from_ini(&ini, "test");
        assert_eq!(c.multicast_addresses, vec!["10.0.0.1", "10.0.0.2"]);
    }

    #[test]
    fn a_hop_count_override_wins() {
        let ini = Ini::parse("[multicast]\nTTLOverride = 7\n").expect("a file");
        assert_eq!(Config::from_ini(&ini, "test").multicast_ttl, 7);
    }

    #[test]
    fn the_protocol_version_never_rises_above_the_one_this_library_speaks() {
        let ini = Ini::parse("[tuning]\nUseProtocolVersion = 200\n").expect("a file");
        assert_eq!(Config::from_ini(&ini, "test").use_protocol_version, 110);
        let ini = Ini::parse("[tuning]\nUseProtocolVersion = 100\n").expect("a file");
        assert_eq!(Config::from_ini(&ini, "test").use_protocol_version, 100);
    }
}
