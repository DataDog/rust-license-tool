use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::{fs, io, io::ErrorKind};

use anyhow::{bail, Context, Result};
use cargo_metadata::{
    DepKindInfo, DependencyKind, MetadataCommand, Node, Package, PackageId, Resolve,
};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

const FILENAME: &str = "license-tool.toml";

const COPYRIGHT_KEY: &str = "__COPYRIGHT__";

#[derive(Default, Deserialize)]
struct Config {
    overrides: Overrides,
}

impl Config {
    fn load() -> Result<Option<Self>> {
        match fs::read_to_string(FILENAME) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("Could not parse {FILENAME:?}"))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Could not load from {FILENAME:?}")),
        }
    }
}

type Overrides = HashMap<String, Override>;

#[derive(Deserialize)]
struct Override {
    license: Option<String>,
    origin: Option<String>,
}

impl Override {
    fn fixup(&self, package: &mut Package) {
        if let Some(license) = &self.license {
            package.license = Some(license.to_owned());
        }
        if let Some(origin) = &self.origin {
            package.repository = Some(origin.to_owned());
        }
    }
}

fn main() -> Result<()> {
    let config = Config::load()?.unwrap_or_default();

    let mut metadata = MetadataCommand::new()
        .verbose(true)
        .exec()
        .context("Running `cargo metadata` failed")?;

    let resolve = metadata
        .resolve
        .context("Metadata is missing a dependency tree")?;
    rewrite_packages(&mut metadata.packages, &config.overrides)?;
    let filtered = filter_deps(resolve);
    let mut packages = lookup_deps(filtered, metadata.packages);
    lookup_all_copyrights(&mut packages)?;
    output_table(packages)
}

// Given a list of package IDs, look up the corresponding entry from the package list and return an
// array of the results.
fn lookup_deps(list: HashSet<PackageId>, packages: Vec<Package>) -> Vec<Package> {
    let mut packages: HashMap<_, _> = packages
        .into_iter()
        .map(|package| (package.id.clone(), package))
        .collect();

    // Use the repository URL as a key to reduce common packages to a single entry
    let mut result = HashMap::<String, Vec<Package>>::new();
    for package in list {
        let package = packages.remove(&package).unwrap();
        let key = package
            .repository
            .clone()
            .unwrap_or_else(|| panic!("Missing repository for {}", package.name));
        result.entry(key).or_insert_with(Vec::new).push(package);
    }
    result.into_iter().map(select_package).collect()
}

// Select out the most likely named package from the set of packages.
fn select_package(item: (String, Vec<Package>)) -> Package {
    let (repository, packages) = item;
    // Convert packages into a map to simplify the lookups
    let mut packages: HashMap<_, _> = packages.into_iter().map(|p| (p.name.clone(), p)).collect();
    // Try the simplest name matching the repository suffix.
    let name = repository.rsplit_once('/').map(|(_, b)| b).unwrap();
    if let Some(package) = packages.remove(name) {
        return package;
    }
    // Try with a common `rust-` prefix
    if let Some(name) = name.strip_prefix("rust-") {
        if let Some(package) = packages.remove(name) {
            return package;
        }
    }
    // Look for a package name ending with or starting with the repository name
    if let Some(key) = packages.keys().find(|key| key.ends_with(name)).cloned() {
        return packages.remove(&key).unwrap();
    }
    if let Some(key) = packages.keys().find(|key| key.starts_with(name)).cloned() {
        return packages.remove(&key).unwrap();
    }
    panic!("Could not determine best package for {repository}");
}

// Filter the list of dependencies to exclude those that would not be distributed in a built
// artifact. i.e. Skip those dependencies that are only used as build or dev dependencies.
fn filter_deps(resolve: Resolve) -> HashSet<PackageId> {
    let deps: HashMap<_, _> = resolve
        .nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect();
    let root = resolve.root.unwrap();

    let mut filtered = HashSet::new();
    filter_deps_rec(&root, &deps, &mut filtered);
    filtered
}

fn filter_deps_rec(
    node: &PackageId,
    deps: &HashMap<PackageId, Node>,
    packages: &mut HashSet<PackageId>,
) {
    let root = deps.get(node).unwrap();
    for node in &root.deps {
        if is_normal_dep(&node.dep_kinds) {
            packages.insert(node.pkg.clone());
            filter_deps_rec(&node.pkg, deps, packages);
        }
    }
}

fn is_normal_dep(kinds: &[DepKindInfo]) -> bool {
    kinds.iter().any(|dep| dep.kind == DependencyKind::Normal)
}

// Dump the resulting CSV table of packages, sorting them by the package name.
fn output_table(mut packages: Vec<Package>) -> Result<()> {
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let mut csv = csv::Writer::from_writer(io::stdout());

    csv.write_record(["Component", "Origin", "License", "Copyright"])?;
    for package in packages {
        // These are fixed up in `rewrite_packages` so we can just `unwrap` with impunity here.
        let repository = package.repository.as_deref().unwrap();
        let license = package.license.as_deref().unwrap();
        let name = package.name;
        let copyright = package
            .metadata
            .get(COPYRIGHT_KEY)
            .unwrap_or_else(|| panic!("Copyright for {name} should have been set"))
            .as_str()
            .expect("Copyright is always set to a string");
        csv.write_record([&name, repository, &license, copyright])?;
    }
    csv.flush().map_err(Into::into)
}

// Rewrite package repository and check presence of licenses
fn rewrite_packages(packages: &mut [Package], overrides: &Overrides) -> Result<()> {
    let mut errors = false;
    for package in packages {
        let name = format!("{}-{}", package.name, package.version);

        if let Some(opts) = overrides.get(&name) {
            opts.fixup(package);
        }

        // Ignore local packages by skipping packages without a source.
        if let Some(source) = &package.source {
            if let Some(repo) = &mut package.repository {
                *repo = strip_git(repo).to_owned();
            } else if let Some(git) = source.repr.strip_prefix("git+") {
                let base = git.find('?').map(|i| &git[..i]).unwrap_or(git);
                package.repository = Some(strip_git(base).to_owned());
            } else {
                eprintln!("Package {name} is missing a repository");
                errors = true;
            }
            if package.license.is_none() {
                eprintln!("Package {name} is missing a license");
                errors = true;
            }
        }
    }
    if errors {
        bail!("Could not fix up package details.")
    } else {
        Ok(())
    }
}

fn strip_git(s: &str) -> &'_ str {
    strip_suffix(strip_suffix(s, ".git"), "/")
}

fn strip_suffix<'a>(s: &'a str, suffix: &str) -> &'a str {
    s.strip_suffix(suffix).unwrap_or(s)
}

// Look through the source files of every package to find something that looks like a copyright
// line, and store the result into the package metadata.
fn lookup_all_copyrights(packages: &mut [Package]) -> Result<()> {
    for package in packages {
        let copyright = Value::String(lookup_copyrights(package)?);
        let key = COPYRIGHT_KEY.to_string();
        match &mut package.metadata {
            Value::Null => {
                package.metadata = Value::Object([(key, copyright)].into_iter().collect())
            }
            Value::Object(map) => {
                map.insert(key, copyright);
            }
            _ => panic!("Package metadata must be an object"),
        }
    }
    Ok(())
}

fn lookup_copyrights(package: &mut Package) -> Result<String> {
    let mut source_path = PathBuf::from(&package.manifest_path);
    source_path.pop();
    if let Some(filename) = &package.license_file {
        if let Some(copyright) = lookup_copyright(filename.as_std_path())? {
            return Ok(copyright);
        }
    }
    for location in COPYRIGHT_LOCATIONS {
        let path = source_path.join(location);
        if path.exists() {
            if let Some(copyright) = lookup_copyright(&path)? {
                return Ok(copyright);
            }
        }
    }
    Ok(if package.authors.is_empty() {
        "NOT FOUND".into()
    } else {
        package.authors.join(", ")
    })
}

fn lookup_copyright(path: &Path) -> Result<Option<String>> {
    let text = fs::read_to_string(path).with_context(|| format!("Could not read {path:?}"))?;
    if let Some(found) = RE_COPYRIGHT.captures(&text) {
        let copyright = &found[0];
        if !RE_COPYRIGHT_IGNORE.is_match(copyright) {
            return Ok(Some(copyright.into()));
        }
    }
    Ok(None)
}

// Files searched for copyright notices
const COPYRIGHT_LOCATIONS: [&str; 17] = [
    "license",
    "LICENSE",
    "license.md",
    "LICENSE.md",
    "LICENSE.txt",
    "License.txt",
    "license.txt",
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "COPYING",
    "NOTICE",
    "README",
    "README.md",
    "README.mdown",
    "README.markdown",
    "COPYRIGHT",
    "COPYRIGHT.txt",
];

// General match for anything that looks like a copyright declaration
static RE_COPYRIGHT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)copyright\s+(?:©|\(c\)\s+)?(?:(?:[0-9 ,-]|present)+\s+)?(?:by\s+)?.*$")
        .unwrap()
});

// Copyright strings to ignore, as they are not owners.  Most of these are from
// boilerplate license files.
//
// These match at the beginning of the copyright (the result of COPYRIGHT_RE).
static RE_COPYRIGHT_IGNORE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
    r"(?i)^(copyright(:? and license)?$|copyright (:?holder|owner|notice|license|statement)|Copyright & License -|copyright .yyyy. .name of copyright owner)").unwrap()
});
