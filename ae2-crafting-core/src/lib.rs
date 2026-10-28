//! Experimental crafting core inspired by AE2 v19.2.18, particularly
//! `appeng/crafting/CraftingCalculation.java` and `execution/CraftingCpuLogic.java`.
//! AE2 is LGPL-3.0-or-later, https://github.com/AppliedEnergistics/Applied-Energistics-2.
//! This is not yet behaviorally equivalent to AE2 and does not access a world.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub mod provider_registry;
pub mod provider_lifecycle;
pub mod emittable_lifecycle;
pub mod cpu_formation;

pub type Inventory = BTreeMap<String, u64>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern { pub id: String, pub output: String, pub output_count: u64, pub inputs: Inventory }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step { pub pattern: String, pub inputs: Inventory, pub output: String, pub count: u64 }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan { pub reserved: Inventory, pub steps: Vec<Step>, pub target: String, pub count: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error { InvalidPattern, DuplicateOutput(String), Missing(String, u64), Cycle(String), Overflow, InventoryChanged(String), Busy, NotWaiting, NoSteps, WrongOutput, AlreadyFinished, SearchLimit, InsufficientCpuBytes { required:u64, available:u64 } }

fn add(map: &mut Inventory, key: &str, n: u64) -> Result<(), Error> {
    let value = map.get(key).copied().unwrap_or(0).checked_add(n).ok_or(Error::Overflow)?;
    if value > 0 { map.insert(key.to_owned(), value); }
    Ok(())
}
fn take(map: &mut Inventory, key: &str, n: u64) -> bool {
    let available = map.get(key).copied().unwrap_or(0);
    if available < n { return false; }
    if available == n { map.remove(key); } else { map.insert(key.to_owned(), available - n); }
    true
}
#[derive(Default)]
pub struct Registry { patterns: BTreeMap<String, Pattern>, ids: BTreeSet<String> }
impl Registry {
    pub fn register(&mut self, pattern: Pattern) -> Result<(), Error> {
        if pattern.id.is_empty() || pattern.output.is_empty() || pattern.output_count == 0 || pattern.inputs.is_empty() || pattern.inputs.iter().any(|(k,v)| k.is_empty() || *v == 0) || self.ids.contains(&pattern.id) { return Err(Error::InvalidPattern); }
        if self.patterns.contains_key(&pattern.output) { return Err(Error::DuplicateOutput(pattern.output)); }
        self.ids.insert(pattern.id.clone()); self.patterns.insert(pattern.output.clone(), pattern); Ok(())
    }
    /// Plans without mutating storage. A submission must recheck the reservation.
    pub fn plan(&self, inventory: &Inventory, target: &str, count: u64) -> Result<Plan, Error> {
        if target.is_empty() || count == 0 { return Err(Error::InvalidPattern); }
        let mut p = Planner { registry: self, original: inventory.clone(), generated: Inventory::new(), reserved: Inventory::new(), steps: vec![], visiting: BTreeSet::new() };
        p.require(target, count)?;
        Ok(Plan { reserved: p.reserved, steps: p.steps, target: target.into(), count })
    }
}
struct Planner<'a> { registry: &'a Registry, original: Inventory, generated: Inventory, reserved: Inventory, steps: Vec<Step>, visiting: BTreeSet<String> }
impl Planner<'_> {
    fn require(&mut self, key: &str, count: u64) -> Result<(), Error> {
        let mut needed = count;
        let from_generated = self.generated.get(key).copied().unwrap_or(0).min(needed);
        take(&mut self.generated, key, from_generated); needed -= from_generated;
        let from_original = self.original.get(key).copied().unwrap_or(0).min(needed);
        take(&mut self.original, key, from_original); add(&mut self.reserved, key, from_original)?; needed -= from_original;
        if needed == 0 { return Ok(()); }
        if !self.visiting.insert(key.into()) { return Err(Error::Cycle(key.into())); }
        let pattern = self.registry.patterns.get(key).ok_or_else(|| Error::Missing(key.into(), needed))?.clone();
        let runs = needed / pattern.output_count + u64::from(needed % pattern.output_count != 0);
        let mut inputs = Inventory::new();
        for (ingredient, amount) in &pattern.inputs {
            let required = amount.checked_mul(runs).ok_or(Error::Overflow)?;
            self.require(ingredient, required)?; inputs.insert(ingredient.clone(), required);
        }
        let produced = pattern.output_count.checked_mul(runs).ok_or(Error::Overflow)?;
        self.steps.push(Step { pattern: pattern.id, inputs, output: key.into(), count: produced });
        add(&mut self.generated, key, produced - needed)?;
        self.visiting.remove(key); Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job { plan: Plan, escrow: Inventory, next: usize, waiting: Option<Step>, finished: bool }
impl Job {
    /// Atomically checks and removes inputs; persist job and inventory atomically in the host.
    pub fn submit(plan: Plan, inventory: &mut Inventory) -> Result<Self, Error> {
        for (k,v) in &plan.reserved { if inventory.get(k).copied().unwrap_or(0) < *v { return Err(Error::InventoryChanged(k.clone())); } }
        for (k,v) in &plan.reserved { take(inventory, k, *v); }
        Ok(Self { escrow: plan.reserved.clone(), plan, next: 0, waiting: None, finished: false })
    }
    /// Remove ingredients from escrow and hand them to a machine. Persist before dispatch;
    /// after a crash reconcile the waiting step with the machine, never blindly dispatch twice.
    pub fn dispatch(&mut self) -> Result<Step, Error> {
        if self.finished { return Err(Error::AlreadyFinished); }
        if self.waiting.is_some() { return Err(Error::Busy); }
        let step = self.plan.steps.get(self.next).cloned().ok_or(Error::NoSteps)?;
        for (k,v) in &step.inputs { if self.escrow.get(k).copied().unwrap_or(0) < *v { return Err(Error::InventoryChanged(k.clone())); } }
        for (k,v) in &step.inputs { take(&mut self.escrow, k, *v); }
        self.waiting = Some(step.clone()); Ok(step)
    }
    /// Machine must confirm exact expected output; external inventories must be reconciled by host.
    pub fn complete_step(&mut self, output: &str, count: u64) -> Result<(), Error> {
        let step = self.waiting.as_ref().ok_or(Error::NotWaiting)?;
        if output != step.output || count != step.count { return Err(Error::WrongOutput); }
        add(&mut self.escrow, output, count)?; self.waiting = None; self.next += 1; Ok(())
    }
    /// Recover a rejected machine dispatch only after host confirms inputs were returned.
    pub fn reject_step(&mut self) -> Result<(), Error> {
        let step = self.waiting.as_ref().ok_or(Error::NotWaiting)?;
        let mut escrow = self.escrow.clone(); for (k,v) in &step.inputs { add(&mut escrow,k,*v)?; }
        self.escrow = escrow; self.waiting = None; Ok(())
    }
    /// Complete returns all produced and surplus items to the caller's storage.
    pub fn finish(&mut self, inventory: &mut Inventory) -> Result<(), Error> {
        if self.finished { return Err(Error::AlreadyFinished); }
        if self.waiting.is_some() { return Err(Error::Busy); }
        if self.next != self.plan.steps.len() { return Err(Error::NoSteps); }
        self.release(inventory)
    }
    /// Cancel only after the in-flight machine has been reconciled.
    pub fn cancel(&mut self, inventory: &mut Inventory) -> Result<(), Error> {
        if self.finished { return Err(Error::AlreadyFinished); }
        if self.waiting.is_some() { return Err(Error::Busy); }
        self.release(inventory)
    }
    fn release(&mut self, inventory: &mut Inventory) -> Result<(), Error> {
        let mut result = inventory.clone(); for (k,v) in &self.escrow { add(&mut result,k,*v)?; }
        *inventory = result; self.escrow.clear(); self.finished = true; Ok(())
    }
    pub fn waiting(&self) -> Option<&Step> { self.waiting.as_ref() }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inv(pairs: &[(&str,u64)]) -> Inventory { pairs.iter().map(|(k,v)| (k.to_string(),*v)).collect() }
    fn pattern(id: &str, out: &str, n: u64, ins: &[(&str,u64)]) -> Pattern { Pattern { id: id.into(), output: out.into(), output_count: n, inputs: inv(ins) } }
    #[test]
    fn nested_batches_surplus_and_persistence() {
        let mut r=Registry::default();
        r.register(pattern("wire", "wire", 4, &[("copper",1)])).unwrap();
        r.register(pattern("circuit", "circuit", 1, &[("wire",3),("iron",1)])).unwrap();
        let mut storage=inv(&[("copper",2),("iron",2)]);
        let plan=r.plan(&storage,"circuit",2).unwrap();
        assert_eq!(plan.reserved, inv(&[("copper",2),("iron",2)]));
        let mut job=Job::submit(plan,&mut storage).unwrap();
        let s=job.dispatch().unwrap(); assert_eq!(s.count,8);
        let encoded=serde_json::to_string(&job).unwrap();
        let mut job: Job=serde_json::from_str(&encoded).unwrap();
        assert_eq!(job.dispatch(), Err(Error::Busy));
        job.complete_step("wire",8).unwrap();
        let s=job.dispatch().unwrap(); assert_eq!(s.inputs,inv(&[("wire",6),("iron",2)]));
        job.complete_step("circuit",2).unwrap(); job.finish(&mut storage).unwrap();
        assert_eq!(storage,inv(&[("circuit",2),("wire",2)]));
        assert_eq!(job.finish(&mut storage),Err(Error::AlreadyFinished));
    }
    #[test]
    fn missing_cycle_and_atomic_submit() {
        let mut r=Registry::default(); r.register(pattern("a","a",1,&[("b",1)])).unwrap();
        assert_eq!(r.plan(&Inventory::new(),"a",1),Err(Error::Missing("b".into(),1)));
        r.register(pattern("b","b",1,&[("a",1)])).unwrap();
        assert_eq!(r.plan(&Inventory::new(),"a",1),Err(Error::Cycle("a".into())));
        let p=r.plan(&inv(&[("b",1)]),"a",1).unwrap();
        let mut changed=inv(&[("other",1)]); let before=changed.clone();
        assert_eq!(Job::submit(p,&mut changed),Err(Error::InventoryChanged("b".into()))); assert_eq!(changed,before);
    }
    #[test]
    fn cancellation_requires_machine_reconciliation() {
        let mut r=Registry::default(); r.register(pattern("p","out",1,&[("in",2)])).unwrap();
        let mut storage=inv(&[("in",2)]); let mut j=Job::submit(r.plan(&storage,"out",1).unwrap(),&mut storage).unwrap();
        j.dispatch().unwrap(); assert_eq!(j.cancel(&mut storage),Err(Error::Busy));
        assert_eq!(j.complete_step("out",2),Err(Error::WrongOutput));
        j.reject_step().unwrap(); j.cancel(&mut storage).unwrap(); assert_eq!(storage,inv(&[("in",2)]));
    }
    #[test]
    fn preexisting_target_and_overflow() {
        let r=Registry::default(); let mut storage=inv(&[("x",5)]);
        let mut j=Job::submit(r.plan(&storage,"x",3).unwrap(),&mut storage).unwrap();
        j.finish(&mut storage).unwrap(); assert_eq!(storage,inv(&[("x",5)]));
        let mut r=Registry::default(); r.register(pattern("p","x",1,&[("y",2)])).unwrap();
        assert_eq!(r.plan(&Inventory::new(),"x",u64::MAX),Err(Error::Overflow));
    }
}

/// Extended experimental planner and multi-operation CPU. Original API remains available.
pub mod extended {
    use super::{add, take, Error, Inventory};
    use serde::{Deserialize, Serialize};
    use std::collections::{BTreeMap, BTreeSet};

    /// One input slot: any one option can satisfy the whole slot per run.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Ingredient { pub choices: Inventory }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Pattern { pub id: String, pub inputs: Vec<Ingredient>, pub outputs: Inventory }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Step { pub pattern: String, pub inputs: Inventory, pub outputs: Inventory, pub runs: u64 }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Plan {
        pub target: String, pub amount: u64, pub reserved: Inventory,
        #[serde(default)] pub emitted: Inventory,
        pub steps: Vec<Step>, pub estimated_bytes: u64,
    }
    #[derive(Clone, Default)]
    pub struct Registry {
        patterns: Vec<Pattern>,
        ids: BTreeSet<String>,
        primary_outputs: BTreeMap<String, String>,
        emittable: BTreeSet<String>,
    }
    impl Registry {
        pub fn register(&mut self, p: Pattern) -> Result<(), Error> {
            if p.id.is_empty() || self.ids.contains(&p.id) || p.inputs.is_empty() || p.outputs.is_empty()
                || p.outputs.iter().any(|(k,v)| k.is_empty() || *v == 0)
                || p.inputs.iter().any(|i| i.choices.is_empty() || i.choices.iter().any(|(k,v)| k.is_empty() || *v == 0)) { return Err(Error::InvalidPattern); }
            self.ids.insert(p.id.clone()); self.patterns.push(p); Ok(())
        }
        /// Index a source definition only by its declared primary output. Its
        /// other outputs still enter the plan as physical byproducts. Existing
        /// callers of `register` retain the legacy all-output lookup behavior.
        pub fn register_primary(&mut self, p: Pattern, primary: String) -> Result<(), Error> {
            if !p.outputs.contains_key(&primary) { return Err(Error::InvalidPattern); }
            let id = p.id.clone();
            self.register(p)?;
            self.primary_outputs.insert(id, primary);
            Ok(())
        }
        /// Snapshot keys from this crafting service's actual provider membership.
        /// Planning records shortages; it never creates or reserves physical items.
        pub fn set_emittable(&mut self, keys: impl IntoIterator<Item=String>) -> Result<(), Error> {
            let keys: BTreeSet<_> = keys.into_iter().collect();
            if keys.contains("") { return Err(Error::InvalidPattern); }
            self.emittable=keys; Ok(())
        }
        /// Tries patterns and alternatives in registration/key order; all mutations are local.
        /// Backtracks across pattern and ingredient alternatives, bounded to
        /// 4096 candidate states and 4096 split-substitution pushes.
        pub fn plan(&self, inventory: &Inventory, target: &str, amount: u64) -> Result<Plan, Error> {
            let mut budget=16_384usize;
            self.plan_with_budget(inventory,target,amount,&mut budget)
        }
        fn plan_with_budget(&self,inventory:&Inventory,target:&str,amount:u64,budget:&mut usize)->Result<Plan,Error> {
            if target.is_empty() || amount == 0 { return Err(Error::InvalidPattern); }
            let state=State { original: inventory.clone(), generated: Inventory::new(), reserved: Inventory::new(), emitted:Inventory::new(), steps: vec![], visiting: BTreeSet::new(), nodes: 0, operations: 0 };
            let mut candidates=self.require_variants(state,target,amount,budget)?;
            if candidates.is_empty(){return Err(Error::Missing(target.into(),amount));}
            let mut state=candidates.remove(0);
            let bytes=state.nodes.checked_mul(8).and_then(|x|x.checked_add(state.operations)).ok_or(Error::Overflow)?;
            Ok(Plan { target: target.into(), amount, reserved: std::mem::take(&mut state.reserved), emitted:state.emitted, steps: state.steps, estimated_bytes: bytes })
        }
        /// A read-only missing-material report. Hypothetical inputs exist
        /// only in this local planning copy; submitting still requires a real
        /// plan against the live inventory.
        pub fn report(&self,inventory:&Inventory,target:&str,amount:u64)->Result<(Plan,Inventory),Error> {
            let mut simulated=inventory.clone();
            // A single shared search budget bounds all hypothetical retries.
            let mut budget=262_144usize;
            for _ in 0..256 {
                match self.plan_with_budget(&simulated,target,amount,&mut budget) {
                    Ok(plan)=>{
                        if plan.steps.is_empty() && plan.emitted.is_empty(){return Err(Error::NoSteps);}
                        let mut missing=Inventory::new();
                        for (key,count) in &plan.reserved {
                            let shortage=count.saturating_sub(inventory.get(key).copied().unwrap_or(0));
                            if shortage>0 {missing.insert(key.clone(),shortage);}
                        }
                        return Ok((plan,missing));
                    }
                    Err(Error::Missing(key,count)) if key!=target && count>0 =>add(&mut simulated,&key,count)?,
                    Err(error)=>return Err(error),
                }
            }
            Err(Error::SearchLimit)
        }
        // Retain multiple locally valid choices until sibling ingredients have
        // also been solved. This permits later conflicts to revisit a pattern
        // selected for an earlier ingredient. A finite budget bounds hostile
        // recipes; large batches with a single choice stay aggregated.
        fn require_variants(&self, mut st: State, key: &str, amount: u64, budget: &mut usize) -> Result<Vec<State>, Error> {
            const LIMIT: usize = 4096;
            if *budget == 0 { return Err(Error::SearchLimit); }
            if st.visiting.len() >= 128 { return Err(Error::SearchLimit); }
            *budget -= 1;
            st.nodes=st.nodes.checked_add(1).ok_or(Error::Overflow)?;
            let from_generated=st.generated.get(key).copied().unwrap_or(0).min(amount);
            take(&mut st.generated,key,from_generated);
            let mut need=amount-from_generated;
            let from_original=st.original.get(key).copied().unwrap_or(0).min(need);
            take(&mut st.original,key,from_original); add(&mut st.reserved,key,from_original)?; need-=from_original;
            if need==0 { return Ok(vec![st]); }
            if self.emittable.contains(key) {add(&mut st.emitted,key,need)?;return Ok(vec![st]);}
            if !st.visiting.insert(key.to_owned()) { return Err(Error::Cycle(key.into())); }
            let mut failure=Error::Missing(key.into(),need);
            let mut answers=Vec::new();
            for p in self.patterns.iter().filter(|p| p.outputs.contains_key(key)
                && self.primary_outputs.get(&p.id).is_none_or(|primary| primary == key)) {
                let count=p.outputs[key]; let runs=need/count + u64::from(need%count != 0);
                let aggregate = self.require_slot_variants(st.clone(), &p.inputs, runs, Inventory::new(), budget);
                let mut variants: Vec<(State, Vec<(Inventory,u64)>)> = match &aggregate {
                    Ok(v) => v.iter().cloned().map(|(state,inputs)|(state,vec![(inputs,runs)])).collect(),
                    Err(e) => { failure=e.clone(); Vec::new() }
                };
                // Include mixed substitutions even when one homogeneous batch
                // succeeds: a later sibling may need some of that resource.
                // Avoid expanding large homogeneous batches into thousands of steps.
                let mixed = p.inputs.iter().any(|i| i.choices.len()>1);
                if runs > 1 && runs <= LIMIT as u64 && (aggregate.is_err() || (mixed && runs <= 16)) {
                    let mut group=vec![(st.clone(),Vec::new())];
                    for _ in 0..runs {
                        let mut next=Vec::new();
                        for (state, selections) in group {
                            match self.require_slot_variants(state, &p.inputs, 1, Inventory::new(), budget) {
                                Ok(options) => for (candidate,inputs) in options {
                                    let mut selected=selections.clone(); selected.push((inputs,1));
                                    next.push((candidate,selected)); if next.len()>=LIMIT { break; }
                                },
                                Err(e)=>failure=e,
                            }
                            if next.len()>=LIMIT { break; }
                        }
                        group=next;
                        if group.is_empty() { break; }
                    }
                    variants.extend(group.into_iter().filter(|(_, selections)| selections.len()==runs as usize));
                }
                for (mut trial,selections) in variants {
                    for (inputs,count_runs) in selections {
                        let mut outputs=Inventory::new();
                        for (output,n) in &p.outputs { let produced=n.checked_mul(count_runs).ok_or(Error::Overflow)?; add(&mut outputs,output,produced)?; add(&mut trial.generated,output,produced)?; }
                        trial.steps.push(Step { pattern:p.id.clone(),inputs,outputs,runs:count_runs });
                    }
                    take(&mut trial.generated,key,need);
                    trial.operations=trial.operations.checked_add(runs).ok_or(Error::Overflow)?;
                    trial.visiting.remove(key);
                    answers.push(trial);
                    if answers.len()>=LIMIT { return Ok(answers); }
                }
            }
            if answers.is_empty() { Err(failure) } else { Ok(answers) }
        }
        fn require_slot_variants(&self, state:State, slots:&[Ingredient], runs:u64, inputs:Inventory, budget: &mut usize) -> Result<Vec<(State,Inventory)>,Error> {
            const LIMIT: usize = 4096;
            if *budget == 0 { return Err(Error::SearchLimit); }
            *budget -= 1;
            let Some((slot,rest))=slots.split_first() else { return Ok(vec![(state,inputs)]); };
            let mut failure=Error::InvalidPattern;
            let mut answers=Vec::new();
            for (candidate,per_run) in &slot.choices {
                let amount=per_run.checked_mul(runs).ok_or(Error::Overflow)?;
                match self.require_variants(state.clone(),candidate,amount,budget) {
                    Ok(variants) => for next in variants {
                        let mut selected=inputs.clone(); add(&mut selected,candidate,amount)?;
                        match self.require_slot_variants(next,rest,runs,selected,budget) {
                            Ok(results)=>{ for answer in results { answers.push(answer); if answers.len()>=LIMIT { return Ok(answers); } } },
                            Err(e)=>failure=e,
                        }
                    },
                    Err(e)=>failure=e,
                }
            }
            if answers.is_empty() { Err(failure) } else { Ok(answers) }
        }

    }
    #[derive(Clone)]
    struct State { original: Inventory, generated: Inventory, reserved: Inventory, emitted:Inventory, steps: Vec<Step>, visiting: BTreeSet<String>, nodes: u64, operations: u64 }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Flight { pub step: Step, pub remaining: Inventory }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Cpu {
        pub capacity_bytes: u64, pub coprocessors: u32,
        plan: Option<Plan>, escrow: Inventory, pending: Vec<Step>, flights: BTreeMap<u64,Flight>, next_id:u64,
        used_ops:[u32;3], cancelled:bool,
        #[serde(default)] emitted_waiting:Inventory,
        #[serde(default)] delivered_target:u64,
    }
    impl Cpu {
        pub fn new(capacity_bytes:u64, coprocessors:u32) -> Self { Self { capacity_bytes,coprocessors,plan:None,escrow:Inventory::new(),pending:vec![],flights:BTreeMap::new(),next_id:0,used_ops:[0;3],cancelled:false,emitted_waiting:Inventory::new(),delivered_target:0 } }
        /// Normalize persisted resource identities atomically across the plan,
        /// physical escrow accounting, pending work, and in-flight work.
        pub fn remap_keys(&mut self, map:impl Fn(&str)->Result<String,String>)->Result<(),String> {
            fn inventory(src:&Inventory,map:&impl Fn(&str)->Result<String,String>)->Result<Inventory,String> {
                let mut out=Inventory::new();for (key,count) in src {let key=map(key)?;let n=out.entry(key).or_default();*n=n.checked_add(*count).ok_or("resource count overflow")?;}Ok(out)
            }
            fn step(s:&mut Step,map:&impl Fn(&str)->Result<String,String>)->Result<(),String> {s.inputs=inventory(&s.inputs,map)?;s.outputs=inventory(&s.outputs,map)?;Ok(())}
            let mut next=self.clone();next.escrow=inventory(&next.escrow,&map)?;
            next.emitted_waiting=inventory(&next.emitted_waiting,&map)?;
            if let Some(plan)=next.plan.as_mut() {plan.target=map(&plan.target)?;plan.reserved=inventory(&plan.reserved,&map)?;plan.emitted=inventory(&plan.emitted,&map)?;for s in &mut plan.steps {step(s,&map)?;}}
            for s in &mut next.pending {step(s,&map)?;}
            for flight in next.flights.values_mut() {step(&mut flight.step,&map)?;flight.remaining=inventory(&flight.remaining,&map)?;}
            *self=next;Ok(())
        }
        pub fn submit(&mut self, plan:Plan, network:&mut Inventory) -> Result<(),Error> {
            if self.plan.is_some() { return Err(Error::Busy); }
            if plan.estimated_bytes > self.capacity_bytes { return Err(Error::InsufficientCpuBytes { required:plan.estimated_bytes, available:self.capacity_bytes }); }
            for (k,v) in &plan.reserved { if network.get(k).copied().unwrap_or(0)<*v { return Err(Error::InventoryChanged(k.clone())); } }
            for (k,v) in &plan.reserved { take(network,k,*v); }
            self.escrow=plan.reserved.clone(); self.pending=plan.steps.clone(); self.emitted_waiting=plan.emitted.clone(); self.plan=Some(plan); self.cancelled=false; self.delivered_target=0; Ok(())
        }
        /// Rotate the AE2 three-tick operation budget once per server tick.
        pub fn tick(&mut self) { self.used_ops[2]=self.used_ops[1]; self.used_ops[1]=self.used_ops[0]; self.used_ops[0]=0; }
        /// Remaining successful pushes in the source's three-tick window.
        /// Inspecting the budget or inputs never reserves items or consumes a medium.
        pub fn remaining_operations(&self) -> u32 {
            if self.plan.is_none() || self.cancelled { return 0; }
            let used=self.used_ops.iter().fold(0u32,|a,b|a.saturating_add(*b));
            self.coprocessors.saturating_add(1).saturating_sub(used)
        }
        pub fn task_has_inputs(&self, index:usize) -> bool {
            let Some(s)=self.pending.get(index) else {return false;};
            s.runs > 0 && s.inputs.iter().all(|(k,n)| *n % s.runs == 0
                && self.escrow.get(k).copied().unwrap_or(0) >= *n / s.runs)
                && s.outputs.values().all(|n| *n % s.runs == 0)
        }
        /// Dispatches one individual runnable push from an aggregated task.
        pub fn dispatch(&mut self) -> Result<(u64,Step),Error> { self.dispatch_where(|_|true) }
        /// Select only a task whose provider has an available destination.
        pub fn dispatch_where(&mut self, mut ready:impl FnMut(&Step)->bool) -> Result<(u64,Step),Error> {
            if self.plan.is_none() || self.cancelled { return Err(Error::AlreadyFinished); }
            if self.remaining_operations()==0 { return Err(Error::Busy); }
            let i=self.pending.iter().enumerate().position(|(i,s)|ready(s) && self.task_has_inputs(i)).ok_or(Error::NoSteps)?;
            self.dispatch_task(i)
        }
        /// Commit the specific task selected by the real provider pass. The
        /// caller owns provider iteration; this method never advances it.
        pub fn dispatch_task(&mut self, i:usize) -> Result<(u64,Step),Error> {
            if self.plan.is_none() || self.cancelled { return Err(Error::AlreadyFinished); }
            if self.remaining_operations()==0 { return Err(Error::Busy); }
            if !self.task_has_inputs(i) { return Err(Error::NoSteps); }
            let id=self.next_id; self.next_id=self.next_id.checked_add(1).ok_or(Error::Overflow)?;
            let pending=&self.pending[i];
            let one=Step { pattern:pending.pattern.clone(), inputs:pending.inputs.iter().map(|(k,n)|(k.clone(),n/pending.runs)).collect(), outputs:pending.outputs.iter().map(|(k,n)|(k.clone(),n/pending.runs)).collect(), runs:1 };
            for (k,n) in &one.inputs { take(&mut self.escrow,k,*n); }
            if self.pending[i].runs==1 { self.pending.remove(i); } else {
                let task=&mut self.pending[i]; task.runs-=1;
                for (k,n) in &one.inputs { take(&mut task.inputs,k,*n); }
                for (k,n) in &one.outputs { take(&mut task.outputs,k,*n); }
            }
            self.flights.insert(id,Flight { remaining:one.outputs.clone(),step:one.clone() }); self.used_ops[0]+=1; Ok((id,one))
        }
        /// Accept only expected output, including partial returns; duplicate delivery is rejected.
        pub fn receive(&mut self, id:u64, key:&str, amount:u64) -> Result<(),Error> {
            let flight=self.flights.get(&id).ok_or(Error::NotWaiting)?;
            if amount==0 || flight.remaining.get(key).copied().unwrap_or(0)<amount { return Err(Error::WrongOutput); }
            let mut escrow=self.escrow.clone(); add(&mut escrow,key,amount)?;
            let f=self.flights.get_mut(&id).unwrap(); take(&mut f.remaining,key,amount); self.escrow=escrow;
            if f.remaining.is_empty() { self.flights.remove(&id); } Ok(())
        }
        /// Source `waitingFor`: emitted shortages plus expected physical
        /// outputs of dispatched patterns. Canceled flights stay in native
        /// custody tracking but no longer count as crafting requests.
        pub fn waiting_for(&self) -> Result<Inventory,Error> {
            let mut result=Inventory::new();
            if self.plan.is_none() || self.cancelled {return Ok(result);}
            for (key,count) in &self.emitted_waiting {add(&mut result,key,*count)?;}
            for flight in self.flights.values() {for (key,count) in &flight.remaining {add(&mut result,key,*count)?;}}
            Ok(result)
        }
        pub fn emitted_waiting(&self) -> &Inventory {&self.emitted_waiting}
        /// Accept only physically delivered items/fluids against an emitted
        /// shortage. The host must separately debit the real source once.
        pub fn receive_emitted(&mut self,key:&str,amount:u64) -> Result<(),Error> {
            if self.plan.is_none() || self.cancelled {return Err(Error::AlreadyFinished);}
            if amount==0 || self.emitted_waiting.get(key).copied().unwrap_or(0)<amount {return Err(Error::WrongOutput);}
            let mut escrow=self.escrow.clone();add(&mut escrow,key,amount)?;
            take(&mut self.emitted_waiting,key,amount);self.escrow=escrow;Ok(())
        }
        /// Request cancellation. In-flight materials stay tracked until reconciled; no further dispatch.
        pub fn cancel(&mut self) -> Result<(),Error> { if self.plan.is_none() { return Err(Error::AlreadyFinished); } self.cancelled=true; self.pending.clear(); self.emitted_waiting.clear(); Ok(()) }
        /// Record target items actually accepted by a physical requester.
        /// Intermediate ingredients or in-flight materials may never be debited.
        pub fn deliver_target(&mut self,key:&str,amount:u64)->Result<(),Error> {
            let plan=self.plan.as_ref().ok_or(Error::AlreadyFinished)?;
            if self.cancelled || !self.pending.is_empty() || !self.flights.is_empty() || !self.emitted_waiting.is_empty() {return Err(Error::Busy);}
            if key!=plan.target || amount==0 || amount>plan.amount.saturating_sub(self.delivered_target) || self.escrow.get(key).copied().unwrap_or(0)<amount {return Err(Error::WrongOutput);}
            self.delivered_target=self.delivered_target.checked_add(amount).ok_or(Error::Overflow)?;
            take(&mut self.escrow,key,amount);
            Ok(())
        }
        /// Recover inputs only after the host proves a dispatch was rejected. Must precede outputs.
        pub fn reject(&mut self,id:u64) -> Result<(),Error> {
            let f=self.flights.get(&id).ok_or(Error::NotWaiting)?; if f.remaining!=f.step.outputs { return Err(Error::WrongOutput); }
            let mut escrow=self.escrow.clone(); for (k,n) in &f.step.inputs { add(&mut escrow,k,*n)?; }
            let f=self.flights.remove(&id).unwrap(); self.escrow=escrow; if !self.cancelled { self.pending.push(f.step); } Ok(())
        }
        /// Transfer all local materials atomically after every in-flight operation is reconciled.
        pub fn finish(&mut self,network:&mut Inventory) -> Result<(),Error> {
            if self.plan.is_none() { return Err(Error::AlreadyFinished); }
            if !self.flights.is_empty() || (!self.cancelled && !self.emitted_waiting.is_empty()) { return Err(Error::Busy); }
            if !self.cancelled && !self.pending.is_empty() { return Err(Error::NoSteps); }
            let mut out=network.clone(); for (k,v) in &self.escrow { add(&mut out,k,*v)?; }
            *network=out; self.plan=None; self.escrow.clear(); self.pending.clear(); self.emitted_waiting.clear(); Ok(())
        }
        pub fn flights(&self)->&BTreeMap<u64,Flight> { &self.flights }
        pub fn escrow(&self)->&Inventory { &self.escrow }
        pub fn pending(&self)->&[Step] { &self.pending }
        /// Remaining final output shown by a crafting monitor. Actual output
        /// already received into physical CPU escrow is no longer pending.
        pub fn display_progress(&self)->Option<(&str,u64)> {
            if self.cancelled {return None;}
            let plan=self.plan.as_ref()?;
            let made=self.escrow.get(&plan.target).copied().unwrap_or(0)
                .saturating_add(self.delivered_target);
            let remaining=plan.amount.saturating_sub(made);
            (remaining>0).then_some((plan.target.as_str(),remaining))
        }
        pub fn is_cancelled(&self)->bool { self.cancelled }
        pub fn is_active(&self)->bool { self.plan.is_some() }
    }
}

#[cfg(test)]
mod extended_tests {
    use super::{extended::*, Error, Inventory};
    fn inv(pairs:&[(&str,u64)])->Inventory { pairs.iter().map(|(k,v)|(k.to_string(),*v)).collect() }
    fn ing(p:&[(&str,u64)])->Ingredient { Ingredient { choices:inv(p) } }
    #[test]
    fn source_primary_output_indexes_byproducts_only_after_the_primary_is_planned() {
        let pattern=Pattern{id:"source".into(),inputs:vec![ing(&[("ore",1)])],outputs:inv(&[("ingot",1),("slag",1)])};
        let mut r=Registry::default();
        assert_eq!(r.register_primary(pattern.clone(),"absent".into()),Err(Error::InvalidPattern));
        r.register_primary(pattern.clone(),"ingot".into()).unwrap();
        assert_eq!(r.plan(&inv(&[("ore",1)]),"slag",1),Err(Error::Missing("slag".into(),1)));
        r.register_primary(Pattern{id:"combine".into(),inputs:vec![ing(&[("ingot",1)]),ing(&[("slag",1)])],outputs:inv(&[("assembly",1)])},"assembly".into()).unwrap();
        let plan=r.plan(&inv(&[("ore",1)]),"assembly",1).unwrap();
        assert_eq!(plan.steps.iter().map(|s|s.pattern.as_str()).collect::<Vec<_>>(),vec!["source","combine"]);
        let mut legacy=Registry::default();legacy.register(pattern).unwrap();
        assert!(legacy.plan(&inv(&[("ore",1)]),"slag",1).is_ok());
    }
    #[test]
    fn cpu_task_inspection_is_non_consuming_and_exact_dispatch_honors_budget() {
        let mut r=Registry::default();
        r.register(Pattern{id:"first".into(),inputs:vec![ing(&[("iron",1)])],outputs:inv(&[("part",1)])}).unwrap();
        r.register(Pattern{id:"second".into(),inputs:vec![ing(&[("part",1)])],outputs:inv(&[("result",1)])}).unwrap();
        let mut network=inv(&[("iron",1)]);let plan=r.plan(&network,"result",1).unwrap();
        let mut cpu=Cpu::new(1024,0);cpu.submit(plan,&mut network).unwrap();
        let before=cpu.clone();assert_eq!(cpu.remaining_operations(),1);assert!(cpu.task_has_inputs(0));assert!(!cpu.task_has_inputs(1));
        assert_eq!(cpu,before);assert_eq!(cpu.dispatch_task(1),Err(Error::NoSteps));assert_eq!(cpu,before);
        let (id,step)=cpu.dispatch_task(0).unwrap();assert_eq!(step.pattern,"first");assert_eq!(cpu.remaining_operations(),0);
        cpu.receive(id,"part",1).unwrap();assert_eq!(cpu.dispatch_task(0),Err(Error::Busy));
        for _ in 0..3 {cpu.tick();}assert!(cpu.task_has_inputs(0));assert_eq!(cpu.dispatch_task(0).unwrap().1.pattern,"second");
    }
    #[test]
    fn emitted_shortage_waits_for_real_receipt_and_survives_reload_and_cancel() {
        let mut r=Registry::default();
        r.register_primary(Pattern{id:"p".into(),inputs:vec![ing(&[("ore",2)])],outputs:inv(&[("diamond",1)])},"diamond".into()).unwrap();
        r.set_emittable(vec!["ore".into()]).unwrap();
        let mut network=inv(&[("ore",1)]);let plan=r.plan(&network,"diamond",2).unwrap();
        assert_eq!(plan.reserved,inv(&[("ore",1)]));assert_eq!(plan.emitted,inv(&[("ore",3)]));
        let mut cpu=Cpu::new(1024,1);cpu.submit(plan,&mut network).unwrap();
        assert!(network.is_empty());assert_eq!(cpu.waiting_for().unwrap(),inv(&[("ore",3)]));
        assert_eq!(cpu.dispatch_task(0),Err(Error::NoSteps));
        assert_eq!(cpu.receive_emitted("ore",4),Err(Error::WrongOutput));
        cpu.receive_emitted("ore",2).unwrap();assert_eq!(cpu.emitted_waiting(),&inv(&[("ore",1)]));
        cpu=serde_json::from_str(&serde_json::to_string(&cpu).unwrap()).unwrap();
        assert_eq!(cpu.waiting_for().unwrap(),inv(&[("ore",1)]));cpu.receive_emitted("ore",1).unwrap();
        let (a,_)=cpu.dispatch_task(0).unwrap();let (b,_)=cpu.dispatch_task(0).unwrap();
        assert_eq!(cpu.waiting_for().unwrap(),inv(&[("diamond",2)]));
        cpu.cancel().unwrap();assert!(cpu.waiting_for().unwrap().is_empty());
        assert_eq!(cpu.finish(&mut network),Err(Error::Busy));
        cpu.receive(a,"diamond",1).unwrap();cpu.receive(b,"diamond",1).unwrap();cpu.finish(&mut network).unwrap();
        assert_eq!(network,inv(&[("diamond",2)]));
    }
    #[test]
    fn emitted_target_is_not_fabricated_and_old_jobs_deserialize() {
        let mut r=Registry::default();r.set_emittable(vec!["diamond".into()]).unwrap();
        let mut network=Inventory::new();let plan=r.plan(&network,"diamond",2).unwrap();
        assert!(plan.steps.is_empty());assert_eq!(plan.emitted,inv(&[("diamond",2)]));
        let mut cpu=Cpu::new(1024,0);cpu.submit(plan,&mut network).unwrap();
        assert_eq!(cpu.finish(&mut network),Err(Error::Busy));assert!(network.is_empty());
        cpu.receive_emitted("diamond",2).unwrap();cpu.finish(&mut network).unwrap();assert_eq!(network,inv(&[("diamond",2)]));
        let old=r.plan(&inv(&[("diamond",1)]),"diamond",1).unwrap();let mut value=serde_json::to_value(old).unwrap();
        value.as_object_mut().unwrap().remove("emitted");
        let restored:Plan=serde_json::from_value(value).unwrap();assert!(restored.emitted.is_empty());
    }
    #[test]
    fn cpu_key_migration_preserves_pending_flights_and_rolls_back_errors() {
        let mut r=Registry::default();r.register(Pattern{id:"one".into(),inputs:vec![ing(&[("old-input",1)])],outputs:inv(&[("old-output",1)])}).unwrap();
        let mut network=inv(&[("old-input",1)]);let plan=r.plan(&network,"old-output",1).unwrap();let mut cpu=Cpu::new(1024,1);cpu.submit(plan,&mut network).unwrap();
        let before=cpu.clone();assert!(cpu.remap_keys(|_|Err("invalid".into())).is_err());assert_eq!(cpu,before);
        let mut in_flight=before.clone();let (flight,_)=in_flight.dispatch().unwrap();in_flight.remap_keys(|s|Ok(s.replace("old-","new-"))).unwrap();in_flight.receive(flight,"new-output",1).unwrap();assert_eq!(in_flight.escrow(),&inv(&[("new-output",1)]));
        cpu.remap_keys(|s|Ok(s.replace("old-","new-"))).unwrap();assert_eq!(cpu.escrow(),&inv(&[("new-input",1)]));
        let (id,step)=cpu.dispatch().unwrap();assert_eq!(step.inputs,inv(&[("new-input",1)]));cpu.receive(id,"new-output",1).unwrap();assert_eq!(cpu.escrow(),&inv(&[("new-output",1)]));
    }
    #[test]
    fn report_explains_multiple_missing_inputs_without_mutating_inventory() {
        let mut r=Registry::default();
        r.register(Pattern{id:"circuit".into(),inputs:vec![ing(&[("iron",2)]),ing(&[("redstone",3)])],outputs:inv(&[("circuit",1)])}).unwrap();
        let network=inv(&[("iron",1)]);
        let (plan,missing)=r.report(&network,"circuit",1).unwrap();
        assert_eq!(network,inv(&[("iron",1)]));
        assert_eq!(missing,inv(&[("iron",1),("redstone",3)]));
        assert_eq!(plan.reserved,inv(&[("iron",2),("redstone",3)]));
        assert!(r.plan(&network,"circuit",1).is_err());
        assert!(r.report(&network,"unregistered",1).is_err());
    }
    #[test]
    fn report_prefers_real_alternative_and_does_not_invent_cycles() {
        let mut r=Registry::default();
        r.register(Pattern{id:"target".into(),inputs:vec![ing(&[("a",1),("b",1)])],outputs:inv(&[("target",1)])}).unwrap();
        let network=inv(&[("b",1)]);
        let (_,missing)=r.report(&network,"target",1).unwrap();
        assert!(missing.is_empty());
        assert_eq!(network,inv(&[("b",1)]));
        let mut cyclic=Registry::default();
        cyclic.register(Pattern{id:"one".into(),inputs:vec![ing(&[("two",1)])],outputs:inv(&[("one",1)])}).unwrap();
        cyclic.register(Pattern{id:"two".into(),inputs:vec![ing(&[("one",1)])],outputs:inv(&[("two",1)])}).unwrap();
        assert!(matches!(cyclic.report(&Inventory::new(),"one",1),Err(Error::Cycle(_))));
    }
    #[test]
    fn target_delivery_debits_real_escrow_across_reload() {
        let mut r=Registry::default();
        r.register(Pattern{id:"planks".into(),inputs:vec![ing(&[("log",1)])],outputs:inv(&[("plank",4)])}).unwrap();
        let mut net=inv(&[("log",1)]);let plan=r.plan(&net,"plank",4).unwrap();
        let mut cpu=Cpu::new(1000,0);cpu.submit(plan,&mut net).unwrap();
        assert_eq!(cpu.display_progress(),Some(("plank",4)));
        assert_eq!(cpu.deliver_target("plank",1),Err(Error::Busy));
        let (flight,_)=cpu.dispatch().unwrap();cpu.receive(flight,"plank",4).unwrap();
        assert_eq!(cpu.display_progress(),None);
        cpu.deliver_target("plank",1).unwrap();assert_eq!(cpu.escrow(),&inv(&[("plank",3)]));
        let mut cpu:Cpu=serde_json::from_str(&serde_json::to_string(&cpu).unwrap()).unwrap();
        cpu.deliver_target("plank",2).unwrap();assert_eq!(cpu.deliver_target("plank",2),Err(Error::WrongOutput));
        cpu.deliver_target("plank",1).unwrap();cpu.finish(&mut net).unwrap();assert!(net.is_empty());
    }
    #[test]
    fn alternatives_byproducts_partial_outputs_and_recovery() {
        let mut r=Registry::default();
        r.register(Pattern{id:"process".into(),inputs:vec![ing(&[("ore",1)])],outputs:inv(&[("plate",1),("dust",1)])}).unwrap();
        r.register(Pattern{id:"circuit".into(),inputs:vec![ing(&[("plate",1)]),ing(&[("dust",1),("expensive",1)])],outputs:inv(&[("circuit",1)])}).unwrap();
        let mut network=inv(&[("ore",2)]); let p=r.plan(&network,"circuit",2).unwrap();
        assert_eq!(p.reserved,inv(&[("ore",2)])); assert_eq!(p.steps.len(),2);
        let mut cpu=Cpu::new(1000,1); cpu.submit(p,&mut network).unwrap();
        let (a,s)=cpu.dispatch().unwrap(); assert_eq!(s.outputs,inv(&[("plate",1),("dust",1)]));
        let (b,_)=cpu.dispatch().unwrap(); cpu.receive(a,"plate",1).unwrap();
        let snap=serde_json::to_string(&cpu).unwrap(); let mut cpu:Cpu=serde_json::from_str(&snap).unwrap();
        cpu.receive(a,"dust",1).unwrap(); cpu.receive(b,"plate",1).unwrap(); cpu.receive(b,"dust",1).unwrap();
        for _ in 0..3 { cpu.tick(); } let (c,_)=cpu.dispatch().unwrap(); let (d,_)=cpu.dispatch().unwrap();
        cpu.receive(c,"circuit",1).unwrap(); cpu.receive(d,"circuit",1).unwrap();
        cpu.finish(&mut network).unwrap(); assert_eq!(network,inv(&[("circuit",2)]));
    }
    #[test]
    fn fallback_cycles_capacity_and_machine_cancellation() {
        let mut r=Registry::default();
        r.register(Pattern{id:"cycle".into(),inputs:vec![ing(&[("a",1)])],outputs:inv(&[("a",1)])}).unwrap();
        r.register(Pattern{id:"valid".into(),inputs:vec![ing(&[("raw",1)])],outputs:inv(&[("a",1),("waste",1)])}).unwrap();
        let mut net=inv(&[("raw",1)]); let p=r.plan(&net,"a",1).unwrap();
        let mut small=Cpu::new(0,0); assert!(matches!(small.submit(p.clone(),&mut net),Err(Error::InsufficientCpuBytes {..}))); assert_eq!(net,inv(&[("raw",1)]));
        let mut cpu=Cpu::new(1000,0); cpu.submit(p,&mut net).unwrap(); let (id,_)=cpu.dispatch().unwrap();
        cpu.cancel().unwrap(); assert_eq!(cpu.finish(&mut net),Err(Error::Busy));
        cpu.receive(id,"a",1).unwrap(); assert_eq!(cpu.reject(id),Err(Error::WrongOutput));
        cpu.receive(id,"waste",1).unwrap(); cpu.finish(&mut net).unwrap(); assert_eq!(net,inv(&[("a",1),("waste",1)]));
    }
    #[test]
    fn independent_flights_and_per_cpu_jobs() {
        let mut r=Registry::default();
        r.register(Pattern{id:"a".into(),inputs:vec![ing(&[("x",1)])],outputs:inv(&[("a",1)])}).unwrap();
        r.register(Pattern{id:"b".into(),inputs:vec![ing(&[("y",1)])],outputs:inv(&[("b",1)])}).unwrap();
        r.register(Pattern{id:"c".into(),inputs:vec![ing(&[("a",1)]),ing(&[("b",1)])],outputs:inv(&[("c",1)])}).unwrap();
        let mut net=inv(&[("x",2),("y",2)]); let p=r.plan(&net,"c",1).unwrap();
        let mut c1=Cpu::new(1000,1); let mut c2=Cpu::new(1000,0); c1.submit(p.clone(),&mut net).unwrap(); c2.submit(p,&mut net).unwrap();
        let (a,_)=c1.dispatch().unwrap(); let (b,_)=c1.dispatch().unwrap(); assert_ne!(a,b); assert_eq!(c1.dispatch(),Err(Error::Busy));
        c1.receive(b,"b",1).unwrap(); c1.receive(a,"a",1).unwrap();
        for _ in 0..3 { c1.tick(); } let (id,_)=c1.dispatch().unwrap(); c1.receive(id,"c",1).unwrap(); c1.finish(&mut net).unwrap();
        assert_eq!(net,inv(&[("c",1)])); c2.cancel().unwrap(); c2.finish(&mut net).unwrap(); assert_eq!(net,inv(&[("c",1),("x",1),("y",1)]));
    }
}

#[cfg(test)]
mod choice_backtracking_test {
    use super::{extended::*, Inventory};
    fn inv(p:&[(&str,u64)])->Inventory { p.iter().map(|(k,v)|(k.to_string(),*v)).collect() }
    #[test]
    fn reserves_rare_item_for_mandatory_slot() {
        let mut r=Registry::default();
        r.register(Pattern{id:"result".into(),inputs:vec![Ingredient{choices:inv(&[("a",1),("b",1)])},Ingredient{choices:inv(&[("a",1)])}],outputs:inv(&[("out",1)])}).unwrap();
        let p=r.plan(&inv(&[("a",1),("b",1)]),"out",1).unwrap();
        assert_eq!(p.steps[0].inputs,inv(&[("a",1),("b",1)]));
    }
    #[test]
    fn revisits_earlier_pattern_when_later_ingredient_needs_shared_stock() {
        let mut r=Registry::default();
        for (id,input) in [("a_from_rare","rare"),("a_from_common","common")] {
            r.register(Pattern{id:id.into(),inputs:vec![Ingredient{choices:inv(&[(input,1)])}],outputs:inv(&[("a",1)])}).unwrap();
        }
        r.register(Pattern{id:"target".into(),inputs:vec![Ingredient{choices:inv(&[("a",1)])},Ingredient{choices:inv(&[("rare",1)])}],outputs:inv(&[("out",1)])}).unwrap();
        let plan=r.plan(&inv(&[("rare",1),("common",1)]),"out",1).unwrap();
        assert!(plan.steps.iter().any(|s|s.pattern=="a_from_common"));
        assert_eq!(plan.reserved,inv(&[("rare",1),("common",1)]));
    }
    #[test]
    fn cyclic_pattern_does_not_hide_noncyclic_alternative() {
        let mut r=Registry::default();
        r.register(Pattern{id:"cycle".into(),inputs:vec![Ingredient{choices:inv(&[("a",1)])}],outputs:inv(&[("a",1)])}).unwrap();
        r.register(Pattern{id:"valid".into(),inputs:vec![Ingredient{choices:inv(&[("raw",1)])}],outputs:inv(&[("a",1)])}).unwrap();
        let plan=r.plan(&inv(&[("raw",1)]),"a",1).unwrap();
        assert_eq!(plan.steps[0].pattern,"valid");
    }
    #[test]
    fn mixed_batch_leaves_shared_input_for_later_recipe() {
        let mut r=Registry::default();
        r.register(Pattern{id:"make_a".into(),inputs:vec![Ingredient{choices:inv(&[("a_rare",1),("b_common",1)])}],outputs:inv(&[("a",1)])}).unwrap();
        r.register(Pattern{id:"target".into(),inputs:vec![Ingredient{choices:inv(&[("a",2)])},Ingredient{choices:inv(&[("a_rare",1)])}],outputs:inv(&[("out",1)])}).unwrap();
        let plan=r.plan(&inv(&[("a_rare",2),("b_common",1)]),"out",1).unwrap();
        assert_eq!(plan.reserved,inv(&[("a_rare",2),("b_common",1)]));
        assert_eq!(plan.steps.iter().filter(|s|s.pattern=="make_a").count(),2);
    }
    #[test]
    fn large_homogeneous_batch_stays_aggregated() {
        let mut r=Registry::default();
        r.register(Pattern{id:"bulk".into(),inputs:vec![Ingredient{choices:inv(&[("raw",1)])}],outputs:inv(&[("out",1)])}).unwrap();
        let p=r.plan(&inv(&[("raw",1_000_000)]),"out",1_000_000).unwrap();
        assert_eq!(p.steps.len(),1);
        assert_eq!(p.steps[0].runs,1_000_000);
    }
}

#[cfg(test)]
mod split_choice_tests {
    use super::{extended::*, Inventory};
    #[test]
    fn substitutions_can_differ_between_actual_pushes() {
        let mut r=Registry::default();
        r.register(Pattern{id:"planks".into(),inputs:vec![Ingredient{choices:Inventory::from([("birch".into(),1),("oak".into(),1)])}],outputs:Inventory::from([("plank".into(),1)])}).unwrap();
        let mut network=Inventory::from([("birch".into(),1),("oak".into(),1)]);
        let plan=r.plan(&network,"plank",2).unwrap();assert_eq!(plan.steps.len(),2);assert_eq!(plan.reserved,network);
        let mut cpu=Cpu::new(100,1);cpu.submit(plan,&mut network).unwrap();let (a,first)=cpu.dispatch().unwrap();let (b,second)=cpu.dispatch().unwrap();
        assert_ne!(first.inputs,second.inputs);cpu.receive(a,"plank",1).unwrap();cpu.receive(b,"plank",1).unwrap();cpu.finish(&mut network).unwrap();assert_eq!(network.get("plank"),Some(&2));
    }
}

#[cfg(test)]
mod push_budget_tests {
    use super::{extended::*,Error,Inventory};
    #[test]
    fn source_three_tick_budget_counts_each_push_and_skips_unavailable_provider() {
        let mut reg=Registry::default();
        reg.register(Pattern{id:"a".into(),inputs:vec![Ingredient{choices:Inventory::from([("x".into(),1)])}],outputs:Inventory::from([("a".into(),1)])}).unwrap();
        reg.register(Pattern{id:"b".into(),inputs:vec![Ingredient{choices:Inventory::from([("y".into(),1)])}],outputs:Inventory::from([("b".into(),1)])}).unwrap();
        reg.register(Pattern{id:"target".into(),inputs:vec![Ingredient{choices:Inventory::from([("a".into(),1)])},Ingredient{choices:Inventory::from([("b".into(),1)])}],outputs:Inventory::from([("z".into(),1)])}).unwrap();
        let mut net=Inventory::from([("x".into(),1),("y".into(),1)]);let plan=reg.plan(&net,"z",1).unwrap();
        let mut cpu=Cpu::new(100,0);cpu.submit(plan,&mut net).unwrap();
        let (id,step)=cpu.dispatch_where(|s|s.pattern=="b").unwrap();assert_eq!(step.pattern,"b");cpu.receive(id,"b",1).unwrap();
        assert_eq!(cpu.dispatch(),Err(Error::Busy));
        cpu.tick();assert_eq!(cpu.dispatch(),Err(Error::Busy));cpu.tick();assert_eq!(cpu.dispatch(),Err(Error::Busy));cpu.tick();
        assert_eq!(cpu.dispatch().unwrap().1.pattern,"a");
    }
}

#[cfg(test)]
mod mixed_output_tests {
    use super::{extended::*, Error, Inventory};
    #[test]
    fn partial_item_and_two_fluid_outputs_are_tracked_independently() {
        let mut r=Registry::default();
        r.register(Pattern{id:"process".into(), inputs:vec![Ingredient{choices:Inventory::from([("ore".into(),1)])}], outputs:Inventory::from([("ingot".into(),1),("fluid:water".into(),500),("fluid:oil".into(),250)])}).unwrap();
        let mut network=Inventory::from([("ore".into(),1)]);
        let mut cpu=Cpu::new(100,0);
        cpu.submit(r.plan(&network,"ingot",1).unwrap(),&mut network).unwrap();
        let (flight,_)=cpu.dispatch().unwrap();
        cpu.receive(flight,"ingot",1).unwrap();
        cpu.receive(flight,"fluid:water",200).unwrap();
        assert_eq!(cpu.finish(&mut network),Err(Error::Busy));
        assert_eq!(cpu.receive(flight,"fluid:water",301),Err(Error::WrongOutput));
        cpu.receive(flight,"fluid:oil",250).unwrap();
        cpu.receive(flight,"fluid:water",300).unwrap();
        cpu.finish(&mut network).unwrap();
        assert_eq!(network.get("fluid:water"),Some(&500));
        assert_eq!(network.get("fluid:oil"),Some(&250));
        assert_eq!(network.get("ingot"),Some(&1));
    }
}
