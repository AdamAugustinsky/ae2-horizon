//! Native external FE adapters and AE2's source debug energy generator.
//! AE2 19.2.18 ForgeEnergyAdapter, EnergyGeneratorBlockEntity (LGPL-3.0+).
//! Default source ratio: 1 FE = 0.5 AE. Private machine buffers are kept
//! separate from the grid's public controller/cell storage.
use super::{Game, ae2_global::GridAddress, ae2_processing};
use horizon_client::{ae2_world::MachineKind, containers::BlockEntity};
use horizon_core::geometry::Direction;
use horizon_world::nbt::Tag;

#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeStorage {
    pub stored: u32,
    pub capacity: u32,
    pub can_receive: bool,
    pub can_extract: bool,
}
const SIDES: [(Direction, [i32; 3]); 6] = [
    (Direction::East, [1, 0, 0]),
    (Direction::West, [-1, 0, 0]),
    (Direction::Up, [0, 1, 0]),
    (Direction::Down, [0, -1, 0]),
    (Direction::South, [0, 0, 1]),
    (Direction::North, [0, 0, -1]),
];
fn as_fe(ae: f64) -> u32 {
    if ae.is_finite() {
        (ae.max(0.0) * 2.0).floor().min(f64::from(i32::MAX)) as u32
    } else {
        0
    }
}
fn generation_rate(tag: Option<&Tag>) -> u32 {
    match tag {
        Some(Tag::Int(n)) => (*n).max(0) as u32,
        _ => 8,
    }
}
fn checked_generation(rate: u32, tier: u32) -> u32 {
    rate.checked_pow(tier)
        .unwrap_or(i32::MAX as u32)
        .min(i32::MAX as u32)
}
impl Game<'_> {
    /// Read-only native capability and propagated-light diagnostics.
    pub(super) fn ae2_power_command(&mut self, args: &[&str]) -> Result<serde_json::Value, String> {
        if args.len() != 4 {
            return Err("usage: /ae2 power x y z face".into());
        }
        let pos = (
            args[0].parse().map_err(|_| "x coordinate")?,
            args[1].parse().map_err(|_| "y coordinate")?,
            args[2].parse().map_err(|_| "z coordinate")?,
        );
        let face = match args[3] {
            "down" => Direction::Down,
            "up" => Direction::Up,
            "north" => Direction::North,
            "south" => Direction::South,
            "west" => Direction::West,
            "east" => Direction::East,
            _ => return Err("block side".into()),
        };
        self.ae2_ensure_machine(pos);
        let address = self.ae2_local_address(pos).ok_or("invalid position")?;
        let packed = self.world.light(pos.0, pos.1, pos.2);
        let energy = match self.ae2_block_entity(address) {
            Some(BlockEntity::Ae2(m)) => Some(m.energy),
            _ => None,
        };
        let fe=self.ae2_fe_storage(address,face).map(|s|serde_json::json!({"stored":s.stored,"capacity":s.capacity,"can_receive":s.can_receive,"can_extract":s.can_extract}));
        let state = self
            .pack
            .assets
            .registry
            .state(self.world.block(pos.0, pos.1, pos.2));
        Ok(
            serde_json::json!({"position":[pos.0,pos.1,pos.2],"energy":energy,"energy_service":self.ae2_energy_storage_diagnostic(address),"fe":fe,"block_light":packed&15,"sky_light":packed>>4,"properties":state.map(|s|&s.properties)}),
        )
    }
    fn ae2_energy_acceptor_face(&self, address: GridAddress, face: Direction) -> bool {
        let Some(BlockEntity::Ae2(m)) = self.ae2_block_entity(address) else {
            return false;
        };
        if m.kind == MachineKind::EnergyAcceptor || m.kind == MachineKind::Controller {
            return true;
        }
        if m.kind != MachineKind::CableBus {
            return false;
        }
        let name = match face {
            Direction::Down => "down",
            Direction::Up => "up",
            Direction::North => "north",
            Direction::South => "south",
            Direction::West => "west",
            Direction::East => "east",
        };
        let Some(Tag::List(parts)) = m.config.get(&"parts".into()) else {
            return false;
        };
        parts.iter().any(|p| {
            p.get("id") == Some(&Tag::String("ae2:cable_energy_acceptor".into()))
                && p.get("face") == Some(&Tag::String(name.into()))
        })
    }
    pub(super) fn ae2_native_fe_storage(
        &self,
        address: GridAddress,
        face: Direction,
    ) -> Option<FeStorage> {
        let Some(BlockEntity::Ae2(m)) = self.ae2_block_entity(address) else {
            return None;
        };
        if m.kind == MachineKind::DebugEnergyGenerator {
            return Some(FeStorage {
                stored: i32::MAX as u32,
                capacity: i32::MAX as u32,
                can_extract: true,
                can_receive: false,
            });
        }
        if self.ae2_energy_acceptor_face(address, face) {
            return Some(FeStorage {
                stored: if m.kind == MachineKind::Controller {
                    as_fe(m.energy)
                } else {
                    0
                },
                capacity: if m.kind == MachineKind::Controller {
                    16000
                } else {
                    0
                },
                can_receive: true,
                can_extract: false,
            });
        }
        let capacity = ae2_processing::ae2_private_power_capacity(m.kind)?;
        let state = self
            .ae2_grid_block(address)
            .and_then(|s| self.pack.assets.registry.state(s))?;
        if !ae2_processing::ae2_private_power_face(m.kind, state, face) {
            return None;
        }
        Some(FeStorage {
            stored: as_fe(m.energy.min(capacity)),
            capacity: as_fe(capacity),
            can_receive: true,
            can_extract: false,
        })
    }
    pub(super) fn ae2_native_fe_extract(
        &mut self,
        address: GridAddress,
        face: Direction,
        amount: u32,
        _simulate: bool,
    ) -> u32 {
        if self
            .ae2_native_fe_storage(address, face)
            .is_some_and(|s| s.can_extract)
        {
            amount.min(i32::MAX as u32)
        } else {
            0
        }
    }
    pub(super) fn ae2_native_fe_receive(
        &mut self,
        address: GridAddress,
        face: Direction,
        amount: u32,
        simulate: bool,
    ) -> u32 {
        if amount == 0 {
            return 0;
        }
        if !self
            .ae2_native_fe_storage(address, face)
            .is_some_and(|s| s.can_receive)
        {
            return 0;
        }
        if self.ae2_energy_acceptor_face(address, face) {
            let part_face=matches!(self.ae2_block_entity(address),Some(BlockEntity::Ae2(machine)) if machine.kind==MachineKind::CableBus).then_some(match face {
                Direction::East=>0,Direction::West=>1,Direction::Up=>2,Direction::Down=>3,Direction::South=>4,Direction::North=>5,
            });
            if let Ok(preview)=self.ae2_energy_transaction(address,part_face,f64::from(amount)*0.5,true,true) {
                let accepted=as_fe(preview).min(amount);
                if !simulate && accepted>0 {
                    match self.ae2_energy_inject_exact(address,part_face,f64::from(accepted)*0.5) {
                        Ok(actual) if as_fe(actual)==accepted=>{},
                        _=>return 0,
                    }
                }
                return accepted;
            }
            let mut network = self.ae2_network();
            let Ok(preview) = network.insert_energy(address.network_pos(), f64::from(amount) * 0.5)
            else {
                return 0;
            };
            // Commit whole FE units only; a partial final FE may not be
            // credited internally after reporting zero to its real source.
            let accepted = as_fe(preview).min(amount);
            if !simulate && accepted > 0 {
                let mut actual = self.ae2_network();
                if actual
                    .insert_energy(address.network_pos(), f64::from(accepted) * 0.5)
                    .is_ok()
                {
                    self.ae2_commit_power(&actual);
                } else {
                    return 0;
                }
            }
            return accepted;
        }
        let room = self
            .ae2_block_entity(address)
            .and_then(|be| match be {
                BlockEntity::Ae2(m) => ae2_processing::ae2_private_power_capacity(m.kind)
                    .map(|cap| as_fe((cap - m.energy).max(0.0))),
                _ => None,
            })
            .unwrap_or(0);
        let accepted = amount.min(room);
        if !simulate && accepted > 0 {
            let Some(BlockEntity::Ae2(m)) = self.ae2_block_entity_mut(address) else {
                return 0;
            };
            m.energy += f64::from(accepted) * 0.5;
            self.ae2_mark_modified(address);
        }
        accepted
    }
    pub(super) fn ae2_tick_external_power(&mut self) {
        let sources: Vec<_> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(&pos, be)| {
                let BlockEntity::Ae2(m) = be else {
                    return None;
                };
                (m.kind == MachineKind::DebugEnergyGenerator)
                    .then_some((pos, generation_rate(m.config.get(&"generationRate".into()))))
            })
            .collect();
        for (pos, rate) in sources {
            let tier=1+SIDES.iter().filter(|(_,d)|matches!(self.world.block_entities.get(&(pos.0+d[0],pos.1+d[1],pos.2+d[2])),Some(BlockEntity::Ae2(m)) if m.kind==MachineKind::DebugEnergyGenerator)).count() as u32;
            let amount = checked_generation(rate, tier);
            for (side, delta) in SIDES {
                let target = (pos.0 + delta[0], pos.1 + delta[1], pos.2 + delta[2]);
                let Some(address) = self.ae2_local_address(target) else {
                    continue;
                };
                let _ = self.ae2_fe_receive(address, side.opposite(), amount, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_fe_scaling_and_integer_boundaries() {
        assert_eq!(as_fe(8.0), 16);
        assert_eq!(as_fe(0.49), 0);
        assert_eq!(checked_generation(8, 3), 512);
        assert_eq!(checked_generation(128, 6), i32::MAX as u32);
        assert_eq!(generation_rate(Some(&Tag::Int(-5))), 0);
    }
}
