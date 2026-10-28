//! Server-owned crafting confirmation for a conversion monitor.
//! A preview is advisory: final submission always replans and reserves the
//! actual current network inventory through the normal CPU path.
use super::{Game, lan::LanHost};
use ae2_cell_core::ItemKey;
use horizon_client::items::ItemStack;
use horizon_protocol::packets::play::{self, Clientbound as C};

pub(super) struct GuestCraftConfirm {
    window: i32,
    state: i32,
    pos: (i32,i32,i32),
    origin: CraftOrigin,
    dimension: String,
    key: ItemKey,
    fluid: bool,
    /// Item counts and fluid amounts are both held in their base units (mB).
    amount: u64,
    craft_missing: bool,
    confirmation: bool,
    cpu: Option<super::ae2_global::GridAddress>,
    report_page: usize,
    status_mode: bool,
    status_cpu: Option<super::ae2_global::GridAddress>,
    status_job: Option<[i64;2]>,
    status_versions: std::collections::VecDeque<(i32,super::ae2_global::GridAddress,Option<[i64;2]>)>,
    status_owner: Option<(super::ae2_parts::PartAddress,(u64,[i32;4]))>,
}
fn craft_amount_text(value: &str, fluid: bool) -> Option<(u64, bool)> {
    let unit = if fluid { 1000 } else { 1 };
    super::ae2_number_entry::number_value(value, unit, 1, i64::from(i32::MAX), '.')
        .map(|n| (n as u64, value.starts_with('=')))
}
fn status_revision_matches(history:&std::collections::VecDeque<(i32,super::ae2_global::GridAddress,Option<[i64;2]>)>,state:i32,cpu:super::ae2_global::GridAddress,job:Option<[i64;2]>)->bool {
    history.iter().rev().find(|(revision,_,_)|*revision==state).is_some_and(|(_,seen_cpu,seen_job)|*seen_cpu==cpu && *seen_job==job)
}
enum CraftOrigin {
    Monitor(String),
    Cpu { clicked:(i32,i32,i32), clicked_incarnation:u64, core_incarnation:u64 },
    Terminal {part:Option<super::ae2_parts::PartAddress>,wireless:Option<super::ae2_wireless::WirelessContext>},
}

impl Game<'_> {
    pub(super) fn lan_open_monitor_craft(&mut self,host:&mut LanHost,conn:u32,pos:(i32,i32,i32),face:&str)->bool {
        let Some(key)=self.ae2_monitor_craft_target(pos,face) else {return false;};
        self.lan_open_craft_confirm(host,conn,pos,key,false,CraftOrigin::Monitor(face.to_owned()))
    }
    pub(super) fn lan_open_terminal_craft_confirm(
        &mut self,host:&mut LanHost,conn:u32,pos:(i32,i32,i32),key:ItemKey,
        fluid:bool,part:Option<super::ae2_parts::PartAddress>,wireless:Option<super::ae2_wireless::WirelessContext>,
    )->bool {
        self.lan_open_craft_confirm(host,conn,pos,key,fluid,CraftOrigin::Terminal{part,wireless})
    }
    pub(super) fn lan_open_cpu_craft_status(&mut self,host:&mut LanHost,conn:u32,pos:(i32,i32,i32))->bool {
        let Some(cpu)=self.ae2_cpu_cluster_owner(pos) else{return false;};
        let Some((dim,core))=self.ae2_address_parts(cpu) else{return false;};
        if dim!=self.dim {return false;}
        let Some(clicked_incarnation)=self.world.block_entities.incarnation(&pos) else{return false;};
        let Some(core_incarnation)=self.world.block_entities.incarnation(&core) else{return false;};
        let Some(guest)=host.hub.guests.iter().find(|g|g.conn==conn && host.hub.here(g)) else{return false;};
        if guest.spectator() || !guest.within_block_range([pos.0,pos.1,pos.2],0.0) {return false;}
        let origin=CraftOrigin::Cpu{clicked:pos,clicked_incarnation,core_incarnation};
        let key=ItemKey{item_id:"minecraft:air".into(),components:vec![]};
        if !self.lan_open_craft_confirm(host,conn,pos,key,false,origin) {return false;}
        let Some(window)=host.hub.open_window(conn,"minecraft:generic_9x3",horizon_protocol::text::literal("AE2 Crafting Status")) else{return true;};
        let identity=self.ae2_recursive_job_identity(cpu);
        if let Some(session)=host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.as_mut()) {session.window=window;session.state=0;session.status_mode=true;session.status_cpu=Some(cpu);session.status_job=identity;}
        self.lan_send_craft_confirm(host,conn);true
    }
    pub(super) fn lan_open_terminal_craft_status(&mut self,host:&mut LanHost,conn:u32,pos:(i32,i32,i32),cpu:Option<super::ae2_global::GridAddress>,part:Option<super::ae2_parts::PartAddress>,wireless:Option<super::ae2_wireless::WirelessContext>,owner:Option<(super::ae2_parts::PartAddress,(u64,[i32;4]))>)->bool {
        let job_id=cpu.and_then(|address|self.ae2_recursive_job_identity(address));
        if cpu.is_some_and(|address|job_id.is_none() && self.ae2_recursive_jobs(pos).iter().any(|job|job.0==address)) {
            let _=self.as_guest(host,conn,|g|g.system_message(horizon_client::text::Text::literal("Crafting job identity is not ready")));return true;
        }
        let key=ItemKey{item_id:"minecraft:air".into(),components:vec![]};
        if !self.lan_open_craft_confirm(host,conn,pos,key,false,CraftOrigin::Terminal{part,wireless}) {return false;}
        let Some(window)=host.hub.open_window(conn,"minecraft:generic_9x3",horizon_protocol::text::literal("AE2 Crafting Status")) else{return true;};
        if let Some(session)=host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.as_mut()) {session.window=window;session.state=0;session.status_mode=true;session.status_cpu=cpu;session.status_job=job_id;session.status_owner=owner;}
        self.lan_send_craft_confirm(host,conn);true
    }
    fn lan_open_craft_confirm(&mut self,host:&mut LanHost,conn:u32,pos:(i32,i32,i32),key:ItemKey,fluid:bool,origin:CraftOrigin)->bool {
        self.lan_pattern_access_closed(host,conn,None);
        self.lan_ae2_network_tool_closed(host,conn,None);self.lan_ae2_anchor_closed(host,conn,None);
        self.lan_ae2_portable_closed(host,conn,None);
        self.lan_ae2_bus_closed(host,conn,None);
        self.lan_ae2_terminal_closed(host,conn,None);
        self.lan_ae2_machine_closed(host,conn,None);
        self.lan_station_closed(host,conn,None);
        self.lan_crafting_closed(host,conn,None);
        self.guest_container_closed(host,conn);
        let Some(window)=host.hub.open_window(conn,"minecraft:generic_9x3",horizon_protocol::text::literal("AE2 Craft Amount")) else {return true;};
        host.play.entry(conn).or_default().ae2_craft_confirm=Some(GuestCraftConfirm {
            window,state:0,pos,origin,dimension:self.dim.id().to_owned(),key,fluid,amount:if fluid {1000} else {1},craft_missing:false,confirmation:false,cpu:None,report_page:0,status_mode:false,status_cpu:None,status_job:None,status_versions:std::collections::VecDeque::new(),status_owner:None,
        });
        self.lan_send_craft_confirm(host,conn);
        true
    }
    fn lan_send_craft_confirm(&mut self,host:&mut LanHost,conn:u32) {
        let Some((window,state,pos,key,fluid,amount,confirmation,cpu,requested_page,status_mode,status_cpu,status_direct))=host.play.get(&conn).and_then(|p|p.ae2_craft_confirm.as_ref()).map(|s|(s.window,s.state,s.pos,s.key.clone(),s.fluid,s.amount,s.confirmation,s.cpu,s.report_page,s.status_mode,s.status_cpu,matches!(s.origin,CraftOrigin::Cpu{..}))) else{return;};
        let mut items=vec![None;27];
        let mut page=0;
        items[0]=if fluid {super::ae2_fluids::fluid_bucket(&key.item_id).map(|id|ItemStack::new(id,1))}
            else {super::ae2_lab::prototype(&key).ok()};
        if let Some(cpu)=status_cpu {
            let rows=self.ae2_recursive_status_rows(cpu).unwrap_or_default();
            let pages=rows.len().max(1).div_ceil(12);page=requested_page.min(pages-1);
            let label=match self.ae2_recursive_suspended(cpu) {Some(true)=>"Suspended",Some(false)=>"Running",None=>"Idle"};
            items[0]=if status_direct {let mut mode=ItemStack::new("minecraft:paper",1);mode.data_mut().custom_name=Some(format!("Selection: {}",self.ae2_cpu_selection_mode(cpu).unwrap_or("invalid")));Some(mode)} else {None};
            let location=self.ae2_address_parts(cpu).map(|(dim,p)|format!("{} {},{},{}",dim.id().strip_prefix("minecraft:").unwrap_or(dim.id()),p.0,p.1,p.2)).unwrap_or_else(||"unknown".into());
            let mut heading=ItemStack::new("minecraft:paper",1);heading.data_mut().custom_name=Some(format!("{}: {}",label,location));items[1]=Some(heading);
            for (i,(key,stored,pending,active)) in rows.into_iter().skip(page*12).take(12).enumerate() {
                let mut line=self.ae2_recursive_status_icon(&key).unwrap_or_else(||ItemStack::new("minecraft:paper",1));
                let name=line.item.to_string();line.count=1;
                line.data_mut().custom_name=Some(format!("{}: stored {}, pending {}, active {}",name,stored,pending,active));items[5+i]=Some(line);
            }
            let mut label=ItemStack::new("minecraft:paper",1);label.data_mut().custom_name=Some(format!("Page {} of {}",page+1,pages));items[17]=Some(label);
            if !status_direct {let mut back=ItemStack::new("minecraft:paper",1);back.data_mut().custom_name=Some("Back to terminal".into());items[20]=Some(back);}
        } else if status_mode {
            items[0]=None;
            let mut heading=ItemStack::new("minecraft:paper",1);heading.data_mut().custom_name=Some("No crafting CPUs".into());items[1]=Some(heading);
            let mut back=ItemStack::new("minecraft:paper",1);back.data_mut().custom_name=Some("Back to terminal".into());items[20]=Some(back);
        } else if confirmation {
            let message=match self.ae2_confirm_report(pos,&key,fluid,amount) {
                Ok((plan,missing))=>{
                    let lines=self.ae2_craft_material_summary(&plan,&missing);
                    let pages=lines.len().max(1).div_ceil(12);
                    page=requested_page.min(pages-1);
                    let mut page_label=ItemStack::new("minecraft:paper",1);
                    page_label.data_mut().custom_name=Some(format!("Page {} of {}",page+1,pages));
                    items[17]=Some(page_label);
                    for (i,label) in lines.into_iter().skip(page*12).take(12).enumerate() {
                        let mut item=ItemStack::new("minecraft:paper",1);
                        item.data_mut().custom_name=Some(label);
                        items[5+i]=Some(item);
                    }
                    if missing.is_empty() {format!("{} {}, {} bytes",plan.steps.len(),if plan.steps.len()==1 {"step"} else {"steps"},plan.estimated_bytes)}
                    else {"Missing materials".into()}
                },
                Err(error)=>error,
            };
            let mut line=ItemStack::new("minecraft:paper",1);
            line.data_mut().custom_name=Some(message);
            items[1]=Some(line);
            let mut selector=ItemStack::new("minecraft:paper",1);
            selector.data_mut().custom_name=Some(cpu.map(|p|format!("CPU: {}",self.ae2_cpu_label(p))).unwrap_or_else(||"CPU: Automatic".into()));
            items[4]=Some(selector);
        }
        let Some(g)=host.hub.guests.iter().find(|g|g.conn==conn) else{return;};
        items.extend((9..36).chain(0..9).map(|i|g.inventory[i].clone()));
        let carried=g.carried.clone();
        let wire_items=items.iter().map(|v|host.hub.stack(v)).collect();
        let wire_carried=host.hub.stack(&carried);
        let live_status_job=status_cpu.and_then(|cpu|self.ae2_recursive_job_identity(cpu));
        let Some(s)=host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.as_mut()) else{return;};
        s.report_page=page;
        s.state=(state+1)&0x7fff;
        let next_state=s.state;
        if status_cpu.is_some() {
            s.status_job=live_status_job;
            s.status_versions.push_back((next_state,status_cpu.unwrap(),live_status_job));
            while s.status_versions.len()>32 {s.status_versions.pop_front();}
        }
        host.hub.send_to(conn,&C::ContainerSetContent(play::ContainerSetContent{container:window,state:next_state,items:wire_items,carried:wire_carried}));
    }
    fn ae2_confirm_preview(&self,pos:(i32,i32,i32),key:&ItemKey,fluid:bool,amount:u64)->Result<ae2_crafting_core::extended::Plan,String> {
        if fluid {self.ae2_preview_recursive_fluid_craft(pos,key,amount)}
        else {self.ae2_preview_recursive_craft(pos,key,amount)}
    }
    fn ae2_confirm_report(&self,pos:(i32,i32,i32),key:&ItemKey,fluid:bool,amount:u64)->Result<(ae2_crafting_core::extended::Plan,ae2_crafting_core::Inventory),String> {
        self.ae2_craft_report(pos,key,fluid,amount)
    }
    pub(super) fn lan_craft_confirm_amount(&mut self,host:&mut LanHost,conn:u32,value:&str)->bool {
        let Some(s)=host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.as_mut()) else{return false;};
        if !s.confirmation {
            // Invalid or empty text cannot silently become an executable job.
            let parsed = craft_amount_text(value, s.fluid);
            s.amount = parsed.map_or(0, |(amount, _)| amount);
            s.craft_missing = parsed.is_some_and(|(_, missing)| missing);
            s.report_page=0;
        }
        true
    }
    pub(super) fn lan_craft_confirm_click(&mut self,host:&mut LanHost,conn:u32,p:&play::ContainerClick)->bool {
        let Some(mut s)=host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.take()) else{return false;};
        if p.container!=s.window {host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);return false;}
        if let CraftOrigin::Terminal{wireless:Some(ctx),..}=&s.origin {
            if self.ae2_wireless_source_valid(host,conn,ctx) && !self.ae2_wireless_validate(host,conn,ctx) {
                // The physical item still exists: keep the screen open while
                // disconnected and permit a later reconnect.
                host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);
                self.lan_send_craft_confirm(host,conn);
                return true;
            }
        }
        let guest=host.hub.guests.iter().find(|g|g.conn==conn && host.hub.here(g));
        let valid=self.dim.id()==s.dimension && guest.is_some() && match &s.origin {
            CraftOrigin::Cpu{clicked,clicked_incarnation,core_incarnation}=>{
                let Some(cpu)=s.status_cpu else{return true;};
                let core=self.ae2_address_parts(cpu).map(|(_,p)|p);
                guest.is_some_and(|g|g.within_block_range([clicked.0,clicked.1,clicked.2],0.0))
                    && self.world.block_entities.incarnation(clicked)==Some(*clicked_incarnation)
                    && core.is_some_and(|p|self.world.block_entities.incarnation(&p)==Some(*core_incarnation))
                    && self.ae2_cpu_cluster_owner(*clicked)==Some(cpu)
            },
            CraftOrigin::Monitor(face)=>guest.is_some_and(|g|g.within_block_range([s.pos.0,s.pos.1,s.pos.2],0.0))
                && self.ae2_monitor_craft_target(s.pos,face).as_ref()==Some(&s.key),
            CraftOrigin::Terminal{part,wireless}=>{
                let access=wireless.as_ref().map_or_else(
                    ||guest.is_some_and(|g|g.within_block_range([s.pos.0,s.pos.1,s.pos.2],4.0)) && self.ae2_has_terminal(s.pos)
                        && part.is_none_or(|p|self.ae2_part_machine(p).ok().flatten().is_some_and(|m|m.kind==horizon_client::ae2_world::MachineKind::CraftingTerminal)),
                    |ctx|self.ae2_wireless_source_valid(host,conn,ctx) && self.ae2_wireless_validate(host,conn,ctx));
                access && (s.status_mode || if s.fluid {self.ae2_craftable_fluids(s.pos).contains(&s.key)} else {self.ae2_craftable_outputs(s.pos).contains(&s.key)})
            }
        };
        if !valid {
            host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));
            return true;
        }
        if s.status_mode {
            let direct=matches!(s.origin,CraftOrigin::Cpu{..});
            if s.status_owner.is_some_and(|(part,identity)|self.ae2_terminal_part_identity(part)!=Some(identity)) {
                host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));return true;
            }
            if super::lan_crafting::input(p.input)!=horizon_client::menu::Input::Pickup || p.button!=0 {
                host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);self.lan_send_craft_confirm(host,conn);return true;
            }
            if p.slot==20 && !direct {
                let origin=s.origin;
                let opened=match origin {
                    CraftOrigin::Terminal{wireless:Some(ctx),..}=>self.lan_open_ae2_wireless_terminal(host,conn,s.pos,ctx),
                    CraftOrigin::Terminal{wireless:None,..}=>match s.status_owner.map(|(part,_)|part) {
                        Some(part)=>match self.ae2_part_machine(part).ok().flatten().map(|m|m.kind) {
                            Some(horizon_client::ae2_world::MachineKind::Terminal)=>self.lan_open_ae2_view_terminal(host,conn,s.pos,part),
                            Some(horizon_client::ae2_world::MachineKind::CraftingTerminal)=>self.lan_open_ae2_crafting_terminal(host,conn,s.pos,part),
                            Some(horizon_client::ae2_world::MachineKind::PatternEncodingTerminal)=>self.lan_open_ae2_encoding_terminal(host,conn,s.pos,part),
                            _=>false,
                        },
                        None=>self.lan_open_ae2_terminal(host,conn,s.pos),
                    },
                    _=>false,
                };
                if !opened {host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));}
                return true;
            }
            let Some(cpu)=s.status_cpu else {
                if p.slot==4 {s.status_cpu=self.ae2_terminal_status_cpus(s.pos).first().copied();}
                host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);self.lan_send_craft_confirm(host,conn);return true;
            };
            if (!direct && !self.ae2_terminal_status_cpus(s.pos).contains(&cpu)) || !status_revision_matches(&s.status_versions,p.state,cpu,self.ae2_recursive_job_identity(cpu)) {
                host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);self.lan_send_craft_confirm(host,conn);return true;
            }
            match p.slot {
                0 if direct=>{let _=self.ae2_cpu_cycle_selection_mode(cpu);},
                2=>{let _=self.ae2_recursive_toggle_suspended(cpu);},
                3=>{let _=self.ae2_recursive_cancel(cpu);},
                4 if !direct=>{let cpus=self.ae2_terminal_status_cpus(s.pos);if let Some(i)=cpus.iter().position(|p|*p==cpu) {let next=cpus[(i+1)%cpus.len()];s.status_cpu=Some(next);s.status_job=self.ae2_recursive_job_identity(next);s.report_page=0;}},
                18=>s.report_page=s.report_page.saturating_sub(1),
                19=>{let pages=self.ae2_recursive_status_rows(cpu).map_or(1,|rows|rows.len().max(1).div_ceil(12));s.report_page=(s.report_page+1).min(pages-1);},
                _=>{}
            }
            host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);self.lan_send_craft_confirm(host,conn);return true;
        }
        // The controls are fixed virtual slots; all other clicks merely
        // resynchronise and never move real inventory.
        if p.slot==2 {
            if !s.confirmation && s.amount>0 {
                if s.craft_missing {
                    // Source's leading '=' requests only what is missing from
                    // the current real network; looking is not a reservation.
                    let contents = if s.fluid { self.ae2_grid_fluids(s.pos) } else { self.ae2_grid_inventory(s.pos) };
                    let Ok(contents) = contents else {
                        host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);
                        self.lan_send_craft_confirm(host,conn);
                        return true;
                    };
                    let existing=contents.iter().find(|(key,_)|key==&s.key).map_or(0,|(_,count)|*count);
                    s.amount=s.amount.saturating_sub(existing.min(i32::MAX as u64));
                    s.craft_missing=false;
                    if s.amount==0 {
                        let opened=match s.origin {
                            CraftOrigin::Terminal{wireless:Some(ctx),..}=>self.lan_open_ae2_wireless_terminal(host,conn,s.pos,ctx),
                            CraftOrigin::Terminal{part:Some(part),wireless:None}=>self.lan_open_ae2_crafting_terminal(host,conn,s.pos,part),
                            CraftOrigin::Terminal{part:None,wireless:None}=>self.lan_open_ae2_terminal(host,conn,s.pos),
                            _=>false,
                        };
                        if !opened {host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));}
                        return true;
                    }
                }
                if let Some(window)=host.hub.open_window(conn,"minecraft:generic_9x3",horizon_protocol::text::literal("AE2 Craft Confirm")) {
                    s.window=window;s.state=0;s.confirmation=true;
                }
            } else if s.confirmation && self.ae2_confirm_preview(s.pos,&s.key,s.fluid,s.amount).is_ok() {
                let power=match &s.origin {CraftOrigin::Terminal{wireless:Some(ctx),..}=>Some(ctx.source),_=>None};
                let result=self.as_guest(host,conn,|game|{
                    let old=game.ae2_wireless_power;game.ae2_wireless_power=power;
                    let result=if s.fluid {
                        if let Some(cpu)=s.cpu {game.ae2_begin_recursive_fluid_craft_on(s.pos,&s.key,s.amount,cpu)}
                        else {game.ae2_begin_recursive_fluid_craft(s.pos,&s.key,s.amount)}
                    } else if let Some(cpu)=s.cpu {game.ae2_begin_recursive_craft_on(s.pos,&s.key,s.amount,cpu)}
                        else {game.ae2_begin_recursive_craft(s.pos,&s.key,s.amount)};
                    game.ae2_wireless_power=old;result
                }).unwrap_or_else(||Err("player unavailable".into()));
                if let Err(error)=result {
                    let _=self.as_guest(host,conn,|g|g.system_message(horizon_client::text::Text::literal(error)));
                } else {
                    host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));
                    return true;
                }
            }
        } else if p.slot==4 && s.confirmation {
            if let Ok((plan,_))=self.ae2_confirm_report(s.pos,&s.key,s.fluid,s.amount) {
                if let Ok(candidates)=self.ae2_craft_cpu_candidates(s.pos,plan.estimated_bytes) {
                    s.cpu=match s.cpu {
                        None=>candidates.first().map(|v|v.0),
                        Some(current)=>candidates.iter().position(|v|v.0==current)
                            .and_then(|i|candidates.get(i+1)).map(|v|v.0),
                    };
                }
            }
        } else if s.confirmation && matches!(p.slot,18|19) {
            if p.slot==18 {s.report_page=s.report_page.saturating_sub(1);}
            else if let Ok((plan,missing))=self.ae2_confirm_report(s.pos,&s.key,s.fluid,s.amount) {
                let pages=self.ae2_craft_material_summary(&plan,&missing).len().max(1).div_ceil(12);
                s.report_page=(s.report_page+1).min(pages-1);
            }
        } else if p.slot==3 {
            host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));
            return true;
        }
        host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);
        self.lan_send_craft_confirm(host,conn);
        true
    }
    pub(super) fn lan_craft_confirm_closed(&mut self,host:&mut LanHost,conn:u32,window:Option<i32>)->bool {
        host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.take_if(|s|window.is_none_or(|w|w==s.window))).is_some()
    }
    pub(super) fn lan_refresh_craft_confirms(&mut self,host:&mut LanHost) {
        let conns:Vec<_>=host.play.iter().filter_map(|(&conn,p)|p.ae2_craft_confirm.as_ref().map(|_|conn)).collect();
        for conn in conns {
            if !host.hub.guests.iter().any(|g|g.conn==conn && host.hub.here(g)){continue;}
            let Some(mut s)=host.play.get_mut(&conn).and_then(|p|p.ae2_craft_confirm.take()) else{continue;};
            if let CraftOrigin::Terminal{wireless:Some(ctx),..}=&mut s.origin {
                if !self.ae2_wireless_source_valid(host,conn,ctx) {
                    host.hub.send_to(conn,&C::ContainerClose(play::ContainerClose{container:s.window}));
                    continue;
                }
                let _=self.ae2_wireless_tick(host,conn,ctx);
            }
            if s.status_mode && matches!(s.origin,CraftOrigin::Terminal{..}) {
                let cpus=self.ae2_terminal_status_cpus(s.pos);
                if s.status_cpu.is_none_or(|cpu|!cpus.contains(&cpu)) {s.status_cpu=cpus.first().copied();s.report_page=0;}
            }
            let refresh_status=s.status_mode && self.ae2_server_tick()%10==0;
            host.play.entry(conn).or_default().ae2_craft_confirm=Some(s);
            if refresh_status {self.lan_send_craft_confirm(host,conn);}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn craft_amount_expression_respects_item_fluid_and_integer_bounds() {
        assert_eq!(craft_amount_text("64*2", false), Some((128, false)));
        assert_eq!(craft_amount_text("=64*2", false), Some((128, true)));
        assert_eq!(craft_amount_text("1/3", false), None);
        assert_eq!(craft_amount_text("0.5", true), Some((500, false)));
        assert_eq!(craft_amount_text("1/3", true), Some((334, false)));
        assert_eq!(craft_amount_text("2147483.647", true), Some((i32::MAX as u64, false)));
        assert_eq!(craft_amount_text("2147483.648", true), None);
        assert_eq!(craft_amount_text("2147483648", false), None);
        assert_eq!(craft_amount_text("0", false), None);
        assert_eq!(craft_amount_text("12345678901234567", false), None);
    }
    #[test]
    fn delayed_status_action_cannot_target_replaced_job_or_another_idle_cpu() {
        let a=super::super::ae2_global::GridAddress::from_local(super::super::Dim::Overworld,(1,64,1)).unwrap();
        let b=super::super::ae2_global::GridAddress::from_local(super::super::Dim::Overworld,(2,64,1)).unwrap();
        let history=std::collections::VecDeque::from([(4,a,Some([1,2])),(5,a,Some([3,4])),(6,b,None)]);
        assert!(status_revision_matches(&history,4,a,Some([1,2])));
        assert!(!status_revision_matches(&history,4,a,Some([3,4])));
        assert!(!status_revision_matches(&history,6,a,None));
        assert!(!status_revision_matches(&history,7,b,None));
    }
}
