use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::mem::take;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use cargo_metadata::{
    DepKindInfo, DependencyKind, MetadataCommand, Node, Package, PackageId, Resolve,
};
use clap::{Parser, Subcommand};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEST_FILENAME: &str = "LICENSE-3rdparty.csv";

const CONFIG_FILENAME: &str = "license-tool.toml";

const COPYRIGHT_KEY: &str = "__COPYRIGHT__";

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

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Dump the generated license data to standard output.
    Dump,
    /// Write the generated license data to the file.
    Write,
    /// Check that the license data is up to date.
    Check,
}

#[derive(Deserialize)]
struct Manifest {
    package: ManifestPackage,
}

#[derive(Deserialize)]
struct ManifestPackage {
    name: String,
}

#[derive(Default, Deserialize)]
struct Config {
    overrides: Overrides,
}

#[derive(Clone, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Record {
    component: String,
    origin: String,
    license: String,
    copyright: String,
}

impl Config {
    fn load() -> Result<Option<Self>> {
        match fs::read_to_string(CONFIG_FILENAME) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("Could not parse {CONFIG_FILENAME:?}")),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("Could not load from {CONFIG_FILENAME:?}"))
            }
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
    let args = Args::parse();
    args.command.doit(build_everything()?)
}

impl Commands {
    fn doit(self, records: Vec<Record>) -> Result<()> {
        match self {
            Self::Dump => output_table(records, io::stdout()),
            Self::Write => Self::write(records),
            Self::Check => Self::check(records),
        }
    }

    fn write(records: Vec<Record>) -> Result<()> {
        let temp_filename = format!("{DEST_FILENAME}.tmp.{}", std::process::id());
        let out = File::create(&temp_filename)
            .with_context(|| format!("Could not create {temp_filename:?}"))?;
        output_table(records, out)?;
        fs::rename(&temp_filename, DEST_FILENAME)
            .with_context(|| format!("Could not rename {temp_filename:?} to {DEST_FILENAME:?}"))
    }

    fn check(records: Vec<Record>) -> Result<()> {
        let mut current: HashMap<String, Record> = match File::open(DEST_FILENAME) {
            Err(error) if error.kind() == ErrorKind::NotFound => Default::default(),
            Err(error) => return Err(error).context(format!("Could not read {DEST_FILENAME:?}")),
            Ok(file) => csv::Reader::from_reader(file)
                .into_deserialize()
                .map(|record| record.map(|record: Record| (record.component.clone(), record)))
                .collect::<Result<_, _>>()
                .with_context(|| format!("Could not read current {DEST_FILENAME:?}"))?,
        };
        let mut errors = false;
        for record in records {
            let component = &record.component;
            if let Some(current) = current.remove(component) {
                if current != record {
                    println!("Record for {component:?} has changed.");
                    errors = true;
                }
            } else {
                println!("Missing record for {component:?}.");
                errors = true;
            }
        }
        if !current.is_empty() {
            for record in current.values() {
                println!("Extraneous record for {:?}.", record.component);
            }
            errors = true;
        }
        if errors {
            bail!("Current {DEST_FILENAME:?} is not up to date.")
        } else {
            Ok(())
        }
    }
}

fn build_everything() -> Result<Vec<Record>> {
    let config = Config::load()?.unwrap_or_default();

    let metadata = MetadataCommand::new()
        .verbose(true)
        .exec()
        .context("Running `cargo metadata` failed")?;

    let resolve = metadata
        .resolve
        .context("Metadata is missing a dependency tree")?;
    let filtered = filter_deps(resolve);
    let mut packages = lookup_deps(filtered, metadata.packages);
    rewrite_packages(&mut packages, &config.overrides)?;
    fixup_names(&mut packages)?;
    lookup_all_copyrights(&mut packages)?;
    Ok(build_records(packages))
}

// Given a list of package IDs, look up the corresponding entry from the package list and return an
// array of the results.
fn lookup_deps(package_ids: HashSet<PackageId>, packages: Vec<Package>) -> Vec<Package> {
    let mut packages: HashMap<_, _> = packages
        .into_iter()
        .map(|package| (package.id.clone(), package))
        .collect();
    package_ids
        .into_iter()
        .map(|id| packages.remove(&id).expect("Missing package {id:?}"))
        .filter(|package| package.source.is_some())
        .collect()
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

// Translate the array of packages into an array of output records.
fn build_records(packages: Vec<Package>) -> Vec<Record> {
    let records = packages.into_iter().map(package_to_record);
    let mut result: Vec<Record> = collect_record_sets(records)
        .into_iter()
        .flat_map(|(record, names)| reduce_names(record, names))
        .collect();
    result.sort_by(|a, b| a.component.cmp(&b.component));
    result
}

// Extract the output record fields from a input package.
fn package_to_record(package: Package) -> Record {
    // These are fixed up in `rewrite_packages` so we can just `unwrap` with impunity here.
    let origin = package.repository.as_deref().unwrap().to_string();
    let license = package.license.as_deref().unwrap().replace('/', " OR ");
    let component = package.name;
    let copyright = package
        .metadata
        .get(COPYRIGHT_KEY)
        .unwrap_or_else(|| panic!("Copyright for {component:?} should have been set"))
        .as_str()
        .expect("Copyright is always set to a string")
        .into();
    Record {
        component,
        origin,
        license,
        copyright,
    }
}

// Collect the given records into sets having identical details except for the component names, which
// are extracted into the hash set value.
fn collect_record_sets(records: impl Iterator<Item = Record>) -> HashMap<Record, HashSet<String>> {
    // Translate the packages into records, and deduplicate nearly identical records that differ
    // only in the component names.
    let mut intermediate: HashMap<Record, HashSet<String>> = HashMap::default();
    for mut record in records {
        let name = take(&mut record.component);
        intermediate
            .entry(record)
            .or_insert_with(HashSet::default)
            .insert(name);
    }
    intermediate
}

// This "rehydrates" the record that is missing a component name into potentially multiple records
// using the set of component names, while attempting to reduce the set down to a single entry.
fn reduce_names(mut record: Record, names: HashSet<String>) -> Vec<Record> {
    if names.len() == 1 {
        record.component = names.into_iter().next().unwrap();
        vec![record]
    } else {
        // If one of the component names matches the repository suffix, use just that one record.
        if let Some((_, suffix)) = record.origin.rsplit_once('/') {
            if names.contains(suffix) {
                record.component = suffix.into();
                return vec![record];
            }
            if let Some(name) = suffix.strip_prefix("rust-") {
                if names.contains(name) {
                    record.component = name.into();
                    return vec![record];
                }
            }
            if let Some(name) = suffix.strip_suffix("-rs") {
                if names.contains(name) {
                    record.component = name.into();
                    return vec![record];
                }
            }
            // More common patterns may be added here as needed to reduce the license file.
        }
        // There is no obvious component name to use as the primary, so just include them all.
        names
            .into_iter()
            .map(|component| Record {
                component,
                ..record.clone()
            })
            .collect()
    }
}

// Dump the resulting CSV table of records.
fn output_table(records: Vec<Record>, writer: impl Write) -> Result<()> {
    let mut csv = csv::Writer::from_writer(writer);
    for record in records {
        csv.serialize(record)?;
    }
    csv.flush().map_err(Into::into)
}

// Rewrite package repository and check presence of licenses
fn rewrite_packages(packages: &mut [Package], overrides: &Overrides) -> Result<()> {
    let errors = packages.iter_mut().fold(false, |errors, package| {
        errors | rewrite_package(package, overrides)
    });
    if errors {
        bail!("Could not fix up package details.")
    } else {
        Ok(())
    }
}

// Rewrite package details, pulling in overrides, to ensure packages with a source also have a
// repository set to `Some`.
fn rewrite_package(package: &mut Package, overrides: &Overrides) -> bool {
    let name = format!("{}-{}", package.name, package.version);

    if let Some(opts) = overrides
        .get(&name)
        .or_else(|| overrides.get(&package.name))
    {
        opts.fixup(package);
    }

    // Don't rewrite local packages by skipping packages without a source.
    if let Some(source) = &package.source {
        if let Some(repo) = &mut package.repository {
            *repo = strip_git(repo).to_owned();
        } else if let Some(git) = source.repr.strip_prefix("git+") {
            let base = git.find('?').map(|i| &git[..i]).unwrap_or(git);
            package.repository = Some(strip_git(base).to_owned());
        } else if let Some(homepage) = package.homepage.clone() {
            package.repository = Some(homepage);
        } else {
            eprintln!("Package {name} is missing a repository");
            return true;
        }
        if package.license.is_none() {
            eprintln!("Package {name} is missing a license");
            return true;
        }
    }
    false
}

fn strip_git(s: &str) -> &'_ str {
    strip_suffix(strip_suffix(s, ".git"), "/")
}

fn strip_suffix<'a>(s: &'a str, suffix: &str) -> &'a str {
    s.strip_suffix(suffix).unwrap_or(s)
}

fn fixup_names(packages: &mut [Package]) -> Result<()> {
    for package in packages {
        let path = &package.manifest_path;
        let text = fs::read_to_string(path)
            .with_context(|| format!("Could not read manifest in {path:?}"))?;
        let manifest: Manifest = toml::from_str(&text)
            .with_context(|| format!("Could not parse manifest in {path:?}"))?;
        package.name = manifest.package.name;
    }
    Ok(())
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
        format!("The {} Authors", package.name)
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
