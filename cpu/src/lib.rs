use std::{collections::HashMap, io::Read, path::Path, sync::LazyLock};

use cxtend::bit_map::BitMap;

#[cfg(not(target_os = "linux"))]
compile_error!("topology-cpu only supports Linux");

pub struct LCore {
    pub package_id: u16,
    pub node_id: u16,
    pub core_id: u16,
    pub lcore_id: u16,
}

pub struct Node {
    pub node_id: u16,
    pub lcores: BitMap,
}

pub struct Package {
    pub package_id: u16,
    pub node_id: u16,
    pub cpu_info: cpuid::CpuInfo,
    pub lcores: BitMap,
}

pub struct Topology {
    pub packages: HashMap<u16, Package>,
    pub lcores: HashMap<u16, LCore>,
    pub nodes: HashMap<u16, Node>,
}

impl Topology {
    fn init() -> Self {
        let mut packages = HashMap::new();
        let mut lcores = HashMap::new();
        let mut nodes = HashMap::new();

        let num_cpus = read_online_from_sysfs("/sys/devices/system/cpu/online");
        let num_nodes = read_online_from_sysfs("/sys/devices/system/node/online");

        for node_id in 0..num_nodes {
            let mut lcores_of_node = BitMap::with_capacity(num_cpus);
            for lcore_id in 0..num_cpus {
                let topology_path = std::path::PathBuf::from(format!(
                    "/sys/devices/system/node/node{}/cpu{}/topology",
                    node_id, lcore_id
                ));

                if !topology_path.exists() {
                    continue;
                }

                let package_id =
                    read_integer_from_sysfs(&topology_path.join("physical_package_id")) as u16;

                let core_id = read_integer_from_sysfs(&topology_path.join("core_id")) as u16;

                lcores.insert(
                    lcore_id as u16,
                    LCore {
                        package_id,
                        node_id: node_id as u16,
                        lcore_id: lcore_id as u16,
                        core_id,
                    },
                );

                packages
                    .entry(package_id)
                    .or_insert(Package {
                        package_id,
                        node_id: node_id as u16,
                        cpu_info: { cpuid::identify_remote(lcore_id as u16).unwrap() },
                        lcores: {
                            let mut lcores_of_package = BitMap::with_capacity(num_cpus);
                            lcores_of_package.set(lcore_id as usize);
                            lcores_of_package
                        },
                    })
                    .lcores
                    .set(lcore_id as usize);

                lcores_of_node.set(lcore_id as usize);
            }

            nodes.insert(
                node_id as u16,
                Node {
                    node_id: node_id as u16,
                    lcores: lcores_of_node,
                },
            );
        }

        Self {
            packages,
            lcores,
            nodes,
        }
    }

    pub fn max_num_nodes(&self) -> u16 {
        self.nodes.len() as u16
    }

    pub fn node_of_lcore(&self, lcore_id: u16) -> Option<&Node> {
        let lcore = self.lcores.get(&lcore_id)?;
        self.nodes.get(&lcore.node_id)
    }

    pub fn node(&self, node_id: u16) -> Option<&Node> {
        self.nodes.get(&node_id)
    }

    pub fn lcores_of_node(&self, node_id: u16) -> Option<&BitMap> {
        Some(&self.nodes.get(&node_id)?.lcores)
    }

    pub fn lcore(&self, lcore_id: u16) -> Option<&LCore> {
        self.lcores.get(&lcore_id)
    }

    pub fn package_of_lcore(&self, lcore_id: u16) -> Option<&Package> {
        let lcore = self.lcores.get(&lcore_id)?;
        self.packages.get(&lcore.package_id)
    }

    pub fn lcores_of_package(&self, package_id: u16) -> Option<&BitMap> {
        Some(&self.packages.get(&package_id)?.lcores)
    }

    pub fn package(&self, package_id: u16) -> Option<&Package> {
        self.packages.get(&package_id)
    }

    pub fn max_num_packages(&self) -> u16 {
        self.packages.len() as u16
    }

    pub fn max_num_lcores(&self) -> u16 {
        self.lcores.len() as u16
    }
}

unsafe impl Send for Topology {}
unsafe impl Sync for Topology {}

static TOPO: LazyLock<Topology> = LazyLock::new(|| Topology::init());

pub fn topology() -> &'static Topology {
    &TOPO
}

fn read_online_from_sysfs<P: AsRef<Path>>(path: P) -> usize {
    let f = std::fs::File::open(path).unwrap();
    let mut reader = std::io::BufReader::new(f);
    let mut buf = String::new();
    reader.read_to_string(&mut buf).unwrap();
    let iter: Vec<&str> = buf.trim().split('-').collect();
    let start = iter[0].parse::<usize>().unwrap();
    let end = iter[1].parse::<usize>().unwrap();
    end - start + 1
}

fn read_integer_from_sysfs<P: AsRef<Path>>(path: P) -> usize {
    let f = std::fs::File::open(path).unwrap();
    let mut reader = std::io::BufReader::new(f);
    let mut buf = String::new();
    reader.read_to_string(&mut buf).unwrap();
    buf.trim().parse::<usize>().unwrap()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_topology() {
        let topo = topology();
        let num_nodes = topo.max_num_nodes();
        let num_packages = topo.max_num_packages();

        for node_id in 0..num_nodes {
            let _ = topo.node(node_id).unwrap();
            let lcores = topo.lcores_of_node(node_id).unwrap();
            for (lcore_id, isset) in lcores.into_iter().enumerate() {
                if !isset {
                    continue;
                }
                let lcore = topo.lcore(lcore_id as u16).unwrap();
                assert_eq!(lcore.node_id, node_id);
                assert_eq!(lcore.lcore_id, lcore_id as u16);
                assert_eq!(
                    lcore.package_id,
                    topo.package_of_lcore(lcore_id as u16).unwrap().package_id
                );
            }

            for package_id in 0..num_packages {
                let package = topo.package(package_id).unwrap();
                let lcores = topo.lcores_of_package(package_id).unwrap();
                for (lcore_id, isset) in lcores.into_iter().enumerate() {
                    if !isset {
                        continue;
                    }
                    let lcore = topo.lcore(lcore_id as u16).unwrap();
                    assert_eq!(lcore.package_id, package.package_id);
                    assert_eq!(lcore.lcore_id, lcore_id as u16);
                    assert_eq!(
                        lcore.node_id,
                        topo.node_of_lcore(lcore_id as u16).unwrap().node_id
                    );
                }
            }
        }
    }
}
