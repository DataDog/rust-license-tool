use std::collections::{HashMap, HashSet};
use std::{borrow::Cow, fs, io::ErrorKind};

use anyhow::{bail, Context, Result};
use cargo_metadata::{
    DepKindInfo, DependencyKind, MetadataCommand, Node, Package, PackageId, Resolve,
};
use serde::Deserialize;

const FILENAME: &str = "license-tool.toml";

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
    let packages = lookup_deps(filtered, metadata.packages);
    output_table(packages);
    Ok(())
}

// Given a list of package IDs, look up the corresponding entry from the package list and return an
// array of the results.
fn lookup_deps(list: HashSet<PackageId>, packages: Vec<Package>) -> Vec<Package> {
    let mut packages: HashMap<_, _> = packages
        .into_iter()
        .map(|package| (package.id.clone(), package))
        .collect();

    // Use the repository URL as a key to reduce common packages
    let mut result = HashMap::<String, Package>::new();
    for package in list {
        let package = packages.remove(&package).unwrap();
        let key = package
            .repository
            .clone()
            // If there is no repository given, use the package name as the unique key.
            .unwrap_or_else(|| format!("UNKNOWN::{}", package.name));
        result.entry(key).or_insert(package);
    }
    result.into_values().collect()
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
fn output_table(mut packages: Vec<Package>) {
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    println!("Component,Origin,License,Copyright");
    for package in packages {
        // These are fixed up in `rewrite_packages` so we can just `unwrap` with impunity here.
        let repository = package.repository.as_deref().unwrap();
        let license = package.license.as_deref().unwrap();
        let copyright = "TODO";
        println!(
            "{},{},{},{}",
            quote(&package.name),
            quote(repository),
            quote(license),
            quote(copyright)
        );
    }
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

fn quote(s: &str) -> Cow<'_, str> {
    if s.contains(',') {
        todo!()
    } else {
        s.into()
    }
}
