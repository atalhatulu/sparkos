//! SparkOS — System Service Supervisor & Daemon Manager (Faz 5.0)
//!
//! Provides Ring-3 Service Manifest Management, Dependency Graph Topological Sorting,
//! Cycle Detection, Self-Healing Restart Policies (Always/OnFailure/Never),
//! Name Resolution & Service Discovery (Name -> Endpoint Capability),
//! Flapping Loop Defense, and Client Reconnect Protocol.

use alloc::vec::Vec;
use alloc::string::String;
use crate::irq_mutex::IrqMutex;
use crate::cap::CapHandle;
pub use spark_abi::service::{
    RestartPolicy, ServiceState, ServiceRequest, ServiceResponse,
    SVC_OP_REGISTER, SVC_OP_LOOKUP, SVC_OP_READY, SVC_OP_HEARTBEAT,
    SVC_OP_STATUS, SVC_OP_RECONNECT, SVC_OP_STOP, MAX_SERVICE_NAME_LEN,
};

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
    pub endpoint_id: Option<u32>,
    pub endpoint_cap: Option<CapHandle>,
    pub generation: u32,
    pub last_heartbeat_tick: u64,
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
    pub init_pid: Option<u64>,
}

pub static SERVICE_MANAGER: IrqMutex<ServiceManager> = IrqMutex::new(ServiceManager::new());

impl ServiceManager {
    pub const fn new() -> Self {
        Self {
            services: Vec::new(),
            init_pid: None,
        }
    }

    pub fn set_init_pid(&mut self, pid: u64) {
        self.init_pid = Some(pid);
    }

    pub fn register(&mut self, def: ServiceDefinition) -> Result<usize, &'static str> {
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
            endpoint_id: None,
            endpoint_cap: None,
            generation: 1,
            last_heartbeat_tick: 0,
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
                        return true;
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

    /// Servis endpoint ve PID kaydı (Ready Handshake)
    pub fn set_service_ready(
        &mut self,
        name: &str,
        pid: u64,
        endpoint_id: u32,
        cap: Option<CapHandle>,
    ) -> Result<(), &'static str> {
        let s = self.services.iter_mut().find(|s| s.def.name == name)
            .ok_or("Service not found")?;
        s.pid = Some(pid);
        s.endpoint_id = Some(endpoint_id);
        s.endpoint_cap = cap;
        s.state = ServiceState::Running;
        Ok(())
    }

    /// Servis Keşfi (Name -> Endpoint ID, Cap, PID, Generation)
    pub fn lookup(&self, name: &str) -> Option<(u32, Option<CapHandle>, u64, u32)> {
        let s = self.services.iter().find(|s| s.def.name == name)?;
        if s.state == ServiceState::Running {
            Some((s.endpoint_id?, s.endpoint_cap, s.pid?, s.generation))
        } else {
            None
        }
    }

    /// Kalp Atışı (Heartbeat)
    pub fn record_heartbeat(&mut self, pid: u64, current_tick: u64) -> bool {
        if let Some(s) = self.services.iter_mut().find(|s| s.pid == Some(pid)) {
            s.last_heartbeat_tick = current_tick;
            true
        } else {
            false
        }
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
        s.generation = s.generation.wrapping_add(1);

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

    /// İstemci yeniden bağlanma (Reconnect Protocol)
    /// Servis yeniden başladığında istemci eski generation'dan yeni generation ve endpoint'e geçer
    pub fn reconnect(&self, name: &str, old_generation: u32) -> Result<(u32, Option<CapHandle>, u64, u32), &'static str> {
        let s = self.services.iter().find(|s| s.def.name == name)
            .ok_or("Service not found")?;
        if s.state != ServiceState::Running {
            return Err("Service not running yet");
        }
        let ep = s.endpoint_id.ok_or("Endpoint missing")?;
        let pid = s.pid.ok_or("PID missing")?;
        if s.generation > old_generation {
            Ok((ep, s.endpoint_cap, pid, s.generation))
        } else {
            // Nesil henüz artmadı veya aynı nesil çalışıyor
            Ok((ep, s.endpoint_cap, pid, s.generation))
        }
    }

    /// IPC İstek Dağıtıcısı (Service Manager IPC Dispatcher)
    pub fn dispatch_request(
        &mut self,
        req: &ServiceRequest,
        attached_cap: Option<CapHandle>,
    ) -> (ServiceResponse, Option<CapHandle>) {
        match req.opcode {
            SVC_OP_REGISTER => {
                let name = req.name_str();
                if let Err(_e) = self.register(ServiceDefinition {
                    name: String::from(name),
                    path: String::from(name),
                    dependencies: Vec::new(),
                    restart_policy: RestartPolicy::Always,
                    is_critical: false,
                    max_retries: 5,
                }) {
                    // Zaten kayıtlı olabilir, endpoint güncelle
                }
                let ep_id = req.extra as u32;
                let _ = self.set_service_ready(name, req.client_pid, ep_id, attached_cap);
                (ServiceResponse::ok(ep_id, req.client_pid, 1, 0), None)
            }
            SVC_OP_LOOKUP => {
                let name = req.name_str();
                if let Some((ep, cap, pid, gen)) = self.lookup(name) {
                    (ServiceResponse::ok(ep, pid, gen, 0), cap)
                } else {
                    (ServiceResponse::err(spark_abi::errno::ENOENT as u32), None)
                }
            }
            SVC_OP_READY => {
                let name = req.name_str();
                let status = req.extra as u32;
                if status == 0 {
                    if let Some(s) = self.services.iter_mut().find(|s| s.def.name == name || s.pid == Some(req.client_pid)) {
                        s.state = ServiceState::Running;
                        return (ServiceResponse::ok(s.endpoint_id.unwrap_or(0), req.client_pid, s.generation, s.restart_count), None);
                    }
                }
                (ServiceResponse::err(spark_abi::errno::EINVAL as u32), None)
            }
            SVC_OP_HEARTBEAT => {
                let tick = crate::interrupts::get_tick();
                if self.record_heartbeat(req.client_pid, tick) {
                    (ServiceResponse::ok(0, req.client_pid, 0, 0), None)
                } else {
                    (ServiceResponse::err(spark_abi::errno::ESRCH as u32), None)
                }
            }
            SVC_OP_RECONNECT => {
                let name = req.name_str();
                let old_gen = req.extra as u32;
                match self.reconnect(name, old_gen) {
                    Ok((ep, cap, pid, gen)) => (ServiceResponse::ok(ep, pid, gen, 0), cap),
                    Err(_) => (ServiceResponse::err(spark_abi::errno::EAGAIN as u32), None),
                }
            }
            _ => (ServiceResponse::err(spark_abi::errno::ENOSYS as u32), None),
        }
    }
}
