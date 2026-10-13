//! SparkOS — System Service Supervisor & Daemon Manager (Faz 19)
//!
//! Provides Ring-3 Service Manifest Management, Dependency Graph Topological Sorting,
//! Cycle Detection, Self-Healing Restart Policies (Always/OnFailure/Never),
//! Flapping Loop Defense, and Reverse-Topological Shutdown.

use alloc::vec::Vec;
use alloc::string::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Restarting,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDefinition {
    pub name: String,
    pub path: String,
    pub dependencies: Vec<String>,
    pub restart_policy: RestartPolicy,
    pub is_critical: bool,
    pub max_retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstance {
    pub id: usize,
    pub def: ServiceDefinition,
    pub state: ServiceState,
    pub pid: Option<u64>,
    pub restart_count: u32,
    pub last_exit_code: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorAction {
    Restart(usize),
    Stop(usize),
    CriticalFailure(usize),
    Ignore,
}

pub struct ServiceManager {
    pub services: Vec<ServiceInstance>,
}

impl ServiceManager {
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
        }
    }

    pub fn register(&mut self, def: ServiceDefinition) -> Result<usize, &'static str> {
        // İsim çakışması kontrolü
        if self.services.iter().any(|s| s.def.name == def.name) {
            return Err("Duplicate service name");
        }

        let id = self.services.len();
        self.services.push(ServiceInstance {
            id,
            def,
            state: ServiceState::Stopped,
            pid: None,
            restart_count: 0,
            last_exit_code: None,
        });
        Ok(id)
    }

    /// Döngüsel bağımlılık (Cycle Detection) kontrolü (DFS)
    pub fn has_cycle(&self) -> bool {
        let n = self.services.len();
        let mut visited = alloc::vec![0u8; n]; // 0: unvisited, 1: visiting, 2: visited

        fn dfs(idx: usize, services: &[ServiceInstance], visited: &mut [u8]) -> bool {
            visited[idx] = 1;

            for dep_name in &services[idx].def.dependencies {
                if let Some(dep_idx) = services.iter().position(|s| &s.def.name == dep_name) {
                    if visited[dep_idx] == 1 {
                        return true; // Döngü tespit edildi!
                    }
                    if visited[dep_idx] == 0 && dfs(dep_idx, services, visited) {
                        return true;
                    }
                }
            }
            visited[idx] = 2;
            false
        }

        for i in 0..n {
            if visited[i] == 0 && dfs(i, &self.services, &mut visited) {
                return true;
            }
        }
        false
    }

    /// Topolojik sıralı başlatma listesi (Boot Order)
    pub fn get_boot_order(&self) -> Result<Vec<usize>, &'static str> {
        if self.has_cycle() {
            return Err("Dependency cycle detected");
        }

        let n = self.services.len();
        let mut visited = alloc::vec![false; n];
        let mut order = Vec::new();

        fn visit(idx: usize, services: &[ServiceInstance], visited: &mut [bool], order: &mut Vec<usize>) {
            if visited[idx] {
                return;
            }
            visited[idx] = true;

            for dep_name in &services[idx].def.dependencies {
                if let Some(dep_idx) = services.iter().position(|s| &s.def.name == dep_name) {
                    visit(dep_idx, services, visited, order);
                }
            }
            order.push(idx);
        }

        for i in 0..n {
            visit(i, &self.services, &mut visited, &mut order);
        }

        Ok(order)
    }

    /// Ters topolojik sıralı kapatma listesi (Shutdown Order)
    pub fn get_shutdown_order(&self) -> Result<Vec<usize>, &'static str> {
        let mut order = self.get_boot_order()?;
        order.reverse();
        Ok(order)
    }

    /// Bir alt süreç sonlandığında süreci değerlendirir ve eylem üretir
    pub fn handle_process_exit(&mut self, pid: u64, exit_code: u64) -> SupervisorAction {
        let service_idx = self.services.iter().position(|s| s.pid == Some(pid));
        let idx = match service_idx {
            Some(i) => i,
            None => return SupervisorAction::Ignore,
        };

        let s = &mut self.services[idx];
        s.last_exit_code = Some(exit_code);
        s.pid = None;

        // Flapping koruması: Max retry aşımı kontrolü
        if s.restart_count >= s.def.max_retries {
            s.state = ServiceState::Failed;
            if s.def.is_critical {
                return SupervisorAction::CriticalFailure(idx);
            }
            return SupervisorAction::Stop(idx);
        }

        match s.def.restart_policy {
            RestartPolicy::Always => {
                s.restart_count += 1;
                s.state = ServiceState::Restarting;
                SupervisorAction::Restart(idx)
            }
            RestartPolicy::OnFailure => {
                if exit_code != 0 {
                    s.restart_count += 1;
                    s.state = ServiceState::Restarting;
                    SupervisorAction::Restart(idx)
                } else {
                    s.state = ServiceState::Stopped;
                    SupervisorAction::Stop(idx)
                }
            }
            RestartPolicy::Never => {
                s.state = ServiceState::Stopped;
                SupervisorAction::Stop(idx)
            }
        }
    }
}
