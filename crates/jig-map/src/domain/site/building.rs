//! Building Site: the project's components, folders, files and types, nested.

use super::*;

impl Site {
    /// The project at `project`, shared out among `domains`, laid out.
    pub fn of(project: &Project, domains: &Domains, show: Show) -> Site {
        let mut site = Site {
            show,
            ..Site::default()
        };
        let name = project
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        site.push(Node {
            key: "dir:".into(),
            kind: Kind::Root,
            title: name.clone(),
            purpose: project.readmes.get(Path::new("")).cloned(),
            detail: String::new(),
            contents: String::new(),
            place: String::new(),
            parent: None,
            children: Vec::new(),
            rect: Rect::default(),
            header: 0.,
            functions: 0,
            open_at: 0.,
            region: 0,
        });
        let (units, homes) = units(project, domains, show);
        // The regions: every domain, and Shared and Unsorted when anything
        // lives there, tests or not.
        let mut regions: Vec<(Home, String, String)> = domains
            .domains
            .iter()
            .enumerate()
            .map(|(ix, domain)| {
                (
                    Home::Domain(ix),
                    format!("domain:{}", domain.name),
                    domain.name.clone(),
                )
            })
            .collect();
        if homes.contains(&Home::Shared) {
            regions.push((Home::Shared, "domain:/shared".into(), "Shared".into()));
        }
        if homes.contains(&Home::Unsorted) {
            regions.push((Home::Unsorted, "domain:/unsorted".into(), "Unsorted".into()));
        }
        let mut leaves: HashMap<(usize, Option<String>), usize> = HashMap::new();
        for (home, key, title) in regions {
            let purpose = match home {
                Home::Domain(ix) => Some(domains.domains[ix].purpose.clone()),
                Home::Shared => Some("Code used everywhere.".into()),
                Home::Unsorted => {
                    Some("New files no domain holds yet. Update Domains places them.".into())
                }
            }
            .filter(|purpose| !purpose.is_empty());
            let region = site.push(Node {
                key,
                kind: Kind::Region(home),
                title,
                purpose,
                detail: String::new(),
                contents: String::new(),
                place: String::new(),
                parent: Some(0),
                children: Vec::new(),
                rect: Rect::default(),
                header: REGION_HEADER,
                functions: 0,
                open_at: 0.,
                region: 0,
            });
            site.nodes[region].region = region;
            let mine: Vec<usize> = (0..units.len())
                .filter(|&u| units[u].home == home)
                .collect();
            site.add_components(project, &units, &mine, region, &name, &mut leaves);
        }
        site.count(0);
        site.size(0);
        site.place_regions();
        site.position(0, 0., 0.);
        site.open_points();
        site.calls = calls(project, &leaves);
        site.by_key = site
            .nodes
            .iter()
            .enumerate()
            .map(|(ix, node)| (node.key.clone(), ix))
            .collect();
        site
    }

    /// Region `region`'s components, each with what's in it.
    pub(super) fn add_components(
        &mut self,
        project: &Project,
        units: &[Unit],
        mine: &[usize],
        region: usize,
        project_name: &str,
        leaves: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        let mut components: BTreeMap<PathBuf, Dir> = BTreeMap::new();
        for &u in mine {
            let path = &units[u].path;
            let component = component_of(project, path);
            let rest = path
                .parent()
                .unwrap_or(Path::new(""))
                .strip_prefix(&component)
                .unwrap_or(Path::new(""));
            let folders: Vec<String> = rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            components.entry(component).or_default().insert(&folders, u);
        }
        for (dir, tree) in &components {
            let mut title = if dir.as_os_str().is_empty() {
                project_name.to_string()
            } else {
                file_name(dir)
            };
            let (mut entries, mut own) = shown(tree, dir);
            // A component holding one folder is that folder's box.
            while own.is_empty() && entries.len() == 1 {
                let (name, path, inner) = entries.remove(0);
                if !BOILERPLATE.contains(&name.as_str()) {
                    title = format!("{title} › {name}");
                }
                (entries, own) = shown(inner, &path);
            }
            let deepest = deepest_place(dir, tree);
            let purpose = project
                .readmes
                .get(dir)
                .cloned()
                .or_else(|| project.readmes.get(Path::new(&deepest)).cloned());
            let ix = self.push(Node {
                key: format!("dir:{region}:{}/", slashed(dir)),
                kind: Kind::Component,
                title,
                purpose,
                detail: String::new(),
                contents: String::new(),
                place: format!("{}/", deepest.trim_end_matches('/')),
                parent: Some(region),
                children: Vec::new(),
                rect: Rect::default(),
                header: TITLE,
                functions: 0,
                open_at: 0.,
                region,
            });
            self.add_entries(project, units, entries, own, ix, region, leaves);
        }
    }

    /// Folders `entries` and units `own` under node `parent`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_entries(
        &mut self,
        project: &Project,
        units: &[Unit],
        entries: Vec<(String, PathBuf, &Dir)>,
        own: &[usize],
        parent: usize,
        region: usize,
        leaves: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        for (name, path, dir) in entries {
            // A folder holding just one file is the file's box.
            if dir.dirs.is_empty() && dir.units.len() == 1 {
                self.add_unit(
                    project,
                    &units[dir.units[0]],
                    Some(&name),
                    parent,
                    region,
                    leaves,
                );
                continue;
            }
            let (inner, inner_own) = shown(dir, &path);
            let place = format!("{}/", slashed(&path));
            let purpose = project
                .readmes
                .get(&path)
                .cloned()
                .or_else(|| folder_doc(project, &path));
            let ix = self.push(Node {
                key: format!("dir:{region}:{place}"),
                kind: Kind::Folder,
                title: name,
                purpose,
                detail: String::new(),
                contents: String::new(),
                place,
                parent: Some(parent),
                children: Vec::new(),
                rect: Rect::default(),
                header: TITLE,
                functions: 0,
                open_at: 0.,
                region,
            });
            self.add_entries(project, units, inner, inner_own, ix, region, leaves);
        }
        for &u in own {
            self.add_unit(project, &units[u], None, parent, region, leaves);
        }
    }

    /// A file or type as a box, titled after the folder it stands for too
    /// when it's that folder's only file; a file's types inside it when
    /// it has several.
    pub(super) fn add_unit(
        &mut self,
        project: &Project,
        unit: &Unit,
        folder: Option<&str>,
        parent: usize,
        region: usize,
        leaves: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        let _ = project;
        let name = match &unit.own_type {
            Some(ty) => ty.clone(),
            None => file_name(&unit.path),
        };
        let title = match folder {
            Some(folder) => format!("{folder} › {name}"),
            None => name,
        };
        let (key, place, kind) = match &unit.own_type {
            Some(ty) => (
                format!("type:{}#{ty}", unit.place),
                format!("{}#{ty}", unit.place),
                Kind::Type,
            ),
            None => (
                format!("file:{}", unit.place),
                unit.place.clone(),
                Kind::File {
                    read: unit.file.is_some(),
                },
            ),
        };
        let ix = self.push(Node {
            key,
            kind,
            title,
            purpose: unit.purpose.clone(),
            detail: String::new(),
            contents: String::new(),
            place,
            parent: Some(parent),
            children: Vec::new(),
            rect: Rect::default(),
            header: TITLE,
            functions: unit.functions,
            open_at: f32::INFINITY,
            region,
        });
        let Some(file) = unit.file else {
            return;
        };
        leaves.insert((file, unit.own_type.clone()), ix);
        // Its types inside it when it has several with code in them.
        let types: Vec<_> = unit.types.iter().filter(|ty| ty.3 > 0).collect();
        if unit.own_type.is_some() || types.len() < 2 {
            return;
        }
        for (ty, kind, purpose, functions) in types {
            let at = self.push(Node {
                key: format!("type:{}#{ty}", unit.place),
                kind: Kind::Type,
                title: format!("{kind} {ty}"),
                purpose: purpose.clone(),
                detail: String::new(),
                contents: String::new(),
                place: format!("{}#{ty}", unit.place),
                parent: Some(ix),
                children: Vec::new(),
                rect: Rect::default(),
                header: TITLE,
                functions: *functions,
                open_at: f32::INFINITY,
                region,
            });
            leaves.insert((file, Some(ty.clone())), at);
        }
    }

    pub(super) fn push(&mut self, node: Node) -> usize {
        let ix = self.nodes.len();
        if let Some(parent) = node.parent {
            self.nodes[parent].children.push(ix);
        }
        self.nodes.push(node);
        ix
    }
}
