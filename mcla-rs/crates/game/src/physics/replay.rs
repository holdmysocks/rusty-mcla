//! Scripted runs of one car on a plane, written out tick by tick, to lay beside a recording of the
//! original game's car (`tools/veh_trace.py`, `tools/veh_compare.py`).
//!
//! `MCLA_VEH_REPLAY="<car>;<upgrade share>;<script>;<out.csv>[;<grade>]"` where the script is a
//! list of `seconds:throttle:brake:steer[:handbrake]` segments, e.g. `0.5:0:0:0,2.5:1:0:0,3:0:1:0`,
//! and the grade is the plane's fall ahead of the car (0.012 = 1.2 % downhill); a sixth field
//! `front,rear` sets the wheel radii.

use super::{CarInput, CarState, SectorMesh, WorldCollider};
use glam::Vec3;
use std::io::Write;
use std::sync::Arc;

pub fn run(data_dir: &str, spec: &str) -> anyhow::Result<()> {
    let f: Vec<&str> = spec.split(';').collect();
    anyhow::ensure!(f.len() >= 4, "MCLA_VEH_REPLAY=<car>;<share>;<script>;<out.csv>[;<grade>]");
    let share: f32 = f[1].parse().unwrap_or(0.0);
    let grade: f32 = f.get(4).and_then(|g| g.parse().ok()).unwrap_or(0.0);
    let assets = crate::assets::Assets::new(data_dir);
    let car = crate::vehicle_model::load_car(&assets, &std::path::Path::new(data_dir).join("resources/vehicle").join(f[0]))?;
    let mut p = crate::vehicles::car_params(&car);
    let Some(st) = car.simtune.as_ref() else { anyhow::bail!("no simulation tuning") };
    let mut tune = mcla_formats::rage::simtune::SimTune::lerp(&st.base, &st.mods, share);
    // (a sixth field "front,rear": the wheel radii of the recorded car - the garage's wheel
    // dimensions change them)
    if let Some(r) = f.get(5) {
        let r: Vec<f32> = r.split(',').filter_map(|x| x.parse().ok()).collect();
        if r.len() >= 2 {
            tune.wheel_front.radius = r[0];
            tune.wheel_back.radius = r[1];
        }
        // (and its suspension offsets, front and rear: the garage's ride height)
        if r.len() >= 4 {
            tune.wheel_front.suspension_offset = r[2];
            tune.wheel_back.suspension_offset = r[3];
        }
        // (and its tyres' widths)
        if r.len() >= 6 {
            tune.wheel_front.width = r[4];
            tune.wheel_back.width = r[5];
        }
    }
    println!("tune bound: friction {} elasticity {} head-on cg offset {} gravity {}", tune.s.bound_friction, tune.s.bound_elasticity, tune.s.head_on_cg_offset, tune.s.bound_gravity);
    println!("tune cg: com {:?} upset {} settle {} drift {} burn xfer {}", (tune.s.com_x, tune.s.com_y, tune.s.com_z), tune.s.upset_cg, tune.s.settle_cg, tune.s.drift_cg, tune.s.burn_weightxfer);
    println!("tune surface drag lat/long front {} {} rear {} {} drift thrust {} decay {}", tune.wheel_front.surface_drag_lat, tune.wheel_front.surface_drag_long, tune.wheel_back.surface_drag_lat, tune.wheel_back.surface_drag_long, tune.gyro.drift_thrust, tune.gyro.drift_decay);
    println!("tune axles front {:?} rear {:?} spans {} {} travel {} {} / {} {}", tune.axle_front, tune.axle_back, tune.wheel_front.suspension_limit + tune.wheel_front.suspension_extent, tune.wheel_back.suspension_limit + tune.wheel_back.suspension_extent, tune.wheel_front.travel_limit, tune.wheel_front.travel_extent, tune.wheel_back.travel_limit, tune.wheel_back.travel_extent);
    println!("tune chassis mass {} com {:?} inertia box {:?} model offset {:?} wheels {:?}", tune.s.mass, (tune.s.com_x, tune.s.com_y, tune.s.com_z), (tune.s.inertia_x, tune.s.inertia_y, tune.s.inertia_z), (tune.s.model_offset_x, tune.s.model_offset_y, tune.s.model_offset_z), p.wheel_pos);
    // (MCLA_REPLAY_ENGAGE=<rpm>: the engine's EngageRPM of a recorded car, where the game changed it)
    if let Some(v) = std::env::var("MCLA_REPLAY_ENGAGE").ok().and_then(|v| v.parse::<f32>().ok()) {
        tune.engine.s.engage_rpm = v;
    }
    // (the body's centre in the model's frame, for the pose a forced state's wheels see - below)
    let pivot_local = Vec3::new(tune.s.model_offset_x, tune.s.model_offset_y, tune.s.model_offset_z);
    println!("tune gyro {:?}", tune.gyro);
    println!("tune axle bone {:?} bits {:?}", tune.axle_bone, tune.axle_bone.map(|m| m.map(|r| r.map(f32::to_bits))));
    println!("tune aero {:?}", tune.aero);
    println!("tune sim {:?}", tune.s);
    println!("tune tirefric {:?} input {:?}", tune.tire_fric, tune.input);
    println!("tune engine {:?}", tune.engine.s);
    println!("tune engine torque nodes {:?} idle nodes {:?}", tune.engine.torque, tune.engine.idle);
    println!("tune trans {:?}", tune.trans.s);
    println!("tune powertrain {:?} freetrain {:?}", tune.powertrain, tune.freetrain);
    println!("tune wheel front {:?}", tune.wheel_front);
    println!("tune wheel back {:?}", tune.wheel_back);
    if let (Some(h), Some(hubs)) = (car.hull.as_ref(), car.hubs) {
        let shape = super::hull::shaped(h, &tune, (hubs[0].y + hubs[2].y) * 0.5, car.bike);
        println!("hull {:?}", shape.verts);
        p.hull = Some(Arc::new(shape));
    }
    let mut vt = super::vehsim::VehTune::new(tune);
    vt.bike = car.bike;
    // (and, from the recording: the tyres' widths, front and rear, and where the wheels sit
    // in the car's frame - half the track front and rear, and their height at rest without
    // the suspension's offset)
    if let Some(r) = f.get(5).map(|r| r.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<f32>>()).filter(|r| r.len() >= 9) {
        for (i, w) in p.wheel_pos.iter_mut().enumerate() {
            w.x = w.x.signum() * if i < 2 { r[6] } else { r[7] };
            w.y = r[8] + p.susp_rest;
        }
        println!("wheels as recorded: widths {:?}, places {:?}", vt.tyre_width, p.wheel_pos);
    }
    println!("shift points {:?} ratios {:?}", vt.shift, vt.ratios);
    if let Some(hubs) = car.hubs {
        vt.seat((hubs[0].y + hubs[2].y) * 0.5);
        println!("seated: com y {}", vt.t.s.com_y);
    }
    p.veh = Some(Arc::new(vt));
    // a seventh field "x,y,z,<nine matrix values>": the city's own ground around that point,
    // the car put down in that pose (rows right, up, back - the chassis matrix of a recording)
    let pose: Vec<f32> = f.get(6).map(|r| r.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    let mut col = WorldCollider::default();
    let dt = 1.0 / 60.0;
    let mut c;
    // (where the recording's car stands before its first pedal)
    let mut start_pose: Option<(Vec3, glam::Quat)> = None;
    if pose.len() == 12 {
        super::collision::load_surfaces(data_dir);
        let at = Vec3::new(pose[0], pose[1], pose[2]);
        // (the city's own collision; MCLA_MESH_COLLISION: the drawn sectors' triangles instead)
        let bound = if std::env::var("MCLA_MESH_COLLISION").is_err() { crate::world::city_bound(&assets) } else { None };
        if let Some(cells) = bound {
            for (i, mesh) in cells.into_iter().enumerate() {
                col.sectors.insert(crate::world::BOUND_ID + i, mesh);
            }
        } else {
            let index = crate::world::WorldIndex::load_or_build(&assets)?;
            for (id, info) in index.sectors.iter().enumerate() {
                let near = at.clamp(Vec3::from(info.min), Vec3::from(info.max));
                if Vec3::new(near.x - at.x, 0.0, near.z - at.z).length() < 250.0 {
                    let cs = crate::world::decode_sector(id, &assets.path(&info.file))?;
                    col.sectors.insert(id, SectorMesh::build_with(cs.collision, cs.collision_mat));
                }
            }
        }
        println!("replay: {} sectors of ground around {:?}", col.sectors.len(), at);
        // (stage P: the level's props, as the physics level holds them. MCLA_DBG_P1: none, as before.)
        if std::env::var_os("MCLA_DBG_P1").is_none() {
            col.loose = super::loose::LooseWorld::load(&assets.path("resources/prop/0x0e66973c.r131"));
            if let Some(l) = col.loose.as_ref() {
                println!("replay: {} prop types, {} instances in the level", l.types.len(), l.instances.len());
            }
        }
        // (stage R: the other vehicles of the recording, their states before every tick from its side file -
        // MCLA_REPLAY_OTHERS, written by veh_diff.py; their bounds from the game's files. MCLA_DBG_RD1: none.
        // With MCLA_REPLAY_STATE (a one-step replay) the pair's kept contacts are the recording's too,
        // unless MCLA_REPLAY_OTHERS_FREE.)
        if let (Ok(p), None) = (std::env::var("MCLA_REPLAY_OTHERS"), std::env::var_os("MCLA_DBG_RD1")) {
            let inject = std::env::var_os("MCLA_REPLAY_STATE").is_some() && std::env::var_os("MCLA_REPLAY_OTHERS_FREE").is_none();
            super::loose::load_others(&p, inject, &|model| {
                let dir = assets.path(&format!("resources/ambients/{model}"));
                let file = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "r155"))?;
                let res = crate::assets::load_resource_file(&file).ok()?;
                let h = mcla_formats::rage::bound::find_hull_of(&res, mcla_formats::rage::bound::AMBIENT_HULL_CLASS)?;
                let mut shape = super::hull::HullShape::new(h.verts.iter().map(|v| Vec3::from(*v)).collect(), &h.tris);
                shape.mats = h.material.clone();
                shape.cg = Vec3::from(h.cg);
                shape.scan = (h.scan as usize).min(shape.verts.len());
                Some(shape)
            });
        }
        c = CarState::new(at, 0.0);
        c.rot = glam::Quat::from_mat3(&glam::Mat3::from_cols(Vec3::new(pose[3], pose[4], pose[5]), Vec3::new(pose[6], pose[7], pose[8]), Vec3::new(pose[9], pose[10], pose[11]))).normalize();
        start_pose = Some((c.pos, c.rot));
    } else {
        // a plane of 16 m tiles, 800 m wide and 6.4 km ahead (forward is -z), falling by `grade`
        let h = |z: f32| z * grade;
        let mut tris = Vec::new();
        for ix in -25..25 {
            for iz in -400..25 {
                let (x0, z0) = (ix as f32 * 16.0, iz as f32 * 16.0);
                let (x1, z1) = (x0 + 16.0, z0 + 16.0);
                tris.push([Vec3::new(x0, h(z0), z0), Vec3::new(x0, h(z1), z1), Vec3::new(x1, h(z0), z0)]);
                tris.push([Vec3::new(x1, h(z0), z0), Vec3::new(x0, h(z1), z1), Vec3::new(x1, h(z1), z1)]);
            }
        }
        col.sectors.insert(0, SectorMesh::build(tris));
        c = CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
    }
    // (A1: a recording does not hold the nitrous count; the replay's button column says when a shot goes)
    c.nitro_shots = 200;
    c.nitro_max = 200;
    // (MCLA_REPLAY_AI: the car is a computer driver's - no boost, no automatic brake)
    super::vehsim::REPLAY_AI.store(std::env::var("MCLA_REPLAY_AI").is_ok(), std::sync::atomic::Ordering::Relaxed);
    for _ in 0..180 {
        c.step(&p, CarInput { handbrake: std::env::var("MCLA_REPLAY_AI").is_ok(), ..Default::default() }, &col, dt);
    }
    println!("replay: settled at {:?} rot {:?}", c.pos, glam::Mat3::from_quat(c.rot));
    // (an eighth field: the engine's torque multiplier the recording starts with - it is what the
    // car last had while it moved)
    if let (Some(m), Some(vs)) = (f.get(7).and_then(|m| m.parse::<f32>().ok()), c.veh.as_mut()) {
        vs.torque_mult = m;
    }
    let mut start = c.pos;
    let mut out = std::io::BufWriter::new(std::fs::File::create(f[3])?);
    writeln!(out, "t,throttle,brake,steer,dist,y,speed,rpm,gear,shift_timer,shaft0,shaft1,diff0,spin0,spin1,spin2,spin3,load0,load1,load2,load3,comp0,comp2,flong0,flong2,slip0,slip2,pedal,torque,drive,melt,boost,moved,heading,flat0,flat2,wheel,side,flat1,flat3,flong1,flong3,yawrate,turn,load1x,load3x,wx,wy,wz,fx,fy,fz,tx,ty,tz,gyt,yaww,sliplat0,sliplat2,displat0,displat2,displong0,displong2,slide0,slide2,roll,lean,pitch,wheelie,vx,vy,vz,avx,avy,avz,comp1,comp3,displat1,displat3,displong1,displong3,px,py,pz,camber0,camber1,camber2,camber3")?;
    let mut before = c.pos;
    let mut t = 0.0f32;
    // the script: segments, or "@file" with one tick a line (throttle brake steer handbrake -
    // what tools/veh_inputs.py writes from a recording of the original)
    let mut ticks: Vec<[f32; 21]> = Vec::new();
    if let Some(path) = f[2].strip_prefix('@') {
        for line in std::fs::read_to_string(path)?.lines() {
            let v: Vec<f32> = line.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 4 {
                // (four more: the surface grip under the wheels, front left/right, rear left/right)
                let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
                // (and four: the roughness height under them)
                // (such a file's steering is what the original's chassis steered with, +132)
                super::vehsim::REPLAY_STEER.store(std::env::var("MCLA_REPLAY_PAD").is_err(), std::sync::atomic::Ordering::Relaxed);
                if v.len() >= 12 {
                    super::vehsim::REPLAY_BUMP_ON.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                // (a thirteenth: the gearbox's way, 1 forward, -1 reverse - the pedals are the chassis's:
                // its throttle drives whichever way the gear points)
                // (and the rider: leaning back 1 .. forward -1, the weight button)
                // (R2, a seventeenth: the damage object's frame update ahead of this tick - 0 none, else one
                // more than the thrown rider's term as the recording logs it, WIPE)
                // (R3, an eighteenth: 1 where the tick's island holds the thrown rider, a jointed body, ISLE;
                // a nineteenth and twentieth: the island's enabled contacts that are not the car's - how
                // many, and their elasticities' sum; a twenty-first: the order the island's sort left the
                // hull's contacts in, as digits - 21 = the second, then the first; 0 = as they stand)
                if v.len() >= 17 && v[16] >= 0.0 {
                    super::vehsim::REPLAY_RIDER_ON.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ticks.push([v[0], v[1], v[2], v[3], g(4), g(5), g(6), g(7), g(8), g(9), g(10), g(11), g(12), g(13), g(14), g(15), g(16), g(17), g(18), g(19), g(20)]);
            }
        }
    } else {
        for seg in f[2].split(',') {
            let v: Vec<f32> = seg.split(':').filter_map(|x| x.parse().ok()).collect();
            anyhow::ensure!(v.len() >= 4, "segment = seconds:throttle:brake:steer[:handbrake]");
            for _ in 0..(v[0] * 60.0).round() as usize {
                ticks.push([v[1], v[2], v[3], v.get(4).copied().unwrap_or(0.0), 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            }
        }
    }
    // (MCLA_REPLAY_POSES: a file with one line a tick - the place and matrix a recording's car
    // had before that tick and its wheels' compressions; the car is put there before each
    // step, to see what this port's collision finds under the original's own wheels)
    let poses: Vec<Vec<f32>> = std::env::var("MCLA_REPLAY_POSES").ok().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.lines().map(|l| l.split_whitespace().filter_map(|x| x.parse().ok()).collect()).collect()).unwrap_or_default();
    let mut states: Vec<Vec<f32>> = std::env::var("MCLA_REPLAY_STATE").ok().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.lines().map(|l| l.split_whitespace().filter_map(|x| x.parse().ok()).collect()).collect()).unwrap_or_default();
    // (MCLA_REPLAY_START=<file>: one line, the whole state of the recording's standing car as
    // its first driven update found it - the values of a MCLA_REPLAY_STATE line. The car is
    // put into it once, before the first tick with a pedal, and runs on by itself: the start
    // of a recording to the bit, where this port's own settled car is only near it. What of
    // the port's own standing car differs from the seed is printed, "START differs".)
    let seeded = states.is_empty() && std::env::var_os("MCLA_REPLAY_START").is_some();
    if seeded {
        let line: Vec<f32> = std::env::var("MCLA_REPLAY_START").ok().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.lines().next().unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
        let first = ticks.iter().position(|v| v[0] > 0.0 || v[1] > 0.0 || v[3] > 0.5 || v[2] != 0.0).unwrap_or(0);
        states = vec![Vec::new(); first];
        states.push(line);
    }
    // (the gearbox's way as the tick AFTER has it: the pad code runs between a tick's vehicle update and its
    // physics step, so the contact callbacks of this tick already find the gear the next update drives in)
    let ways: Vec<i32> = ticks.iter().map(|v| v[12] as i32).collect();
    for (n, v) in ticks.into_iter().enumerate() {
        // (MCLA_DBG_STAMPS: the tick before the damage map stamps it queues, damagemap.rs)
        if let Some(p) = std::env::var_os("MCLA_DBG_STAMPS") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "T {n}");
            }
        }
        super::loose::REPLAY_TICK.store(n, std::sync::atomic::Ordering::Relaxed);
        super::loose::REPLAY_K.store(n as i64 - 30, std::sync::atomic::Ordering::Relaxed);
        super::vehsim::REPLAY_WAY_AFTER.store(ways.get(n + 1).copied().unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
        if let Some(q) = poses.get(n).filter(|q| q.len() >= 16) {
            // (and the way it came: where the recording had it a tick before)
            if let Some(b) = n.checked_sub(1).and_then(|m| poses.get(m)).filter(|b| b.len() >= 16) {
                c.vel = (Vec3::new(q[0], q[1], q[2]) - Vec3::new(b[0], b[1], b[2])) / dt;
            }
            c.pos = Vec3::new(q[0], q[1], q[2]);
            c.rot = glam::Quat::from_mat3(&glam::Mat3::from_cols(Vec3::new(q[3], q[4], q[5]), Vec3::new(q[6], q[7], q[8]), Vec3::new(q[9], q[10], q[11]))).normalize();
            if let Some(vs) = c.veh.as_mut() {
                for i in 0..4 {
                    // (the wheels' shapes from this state, not from the port's last update)
                    vs.wheels[i].part = None;
                    vs.wheels[i].comp = q[12 + i];
                    if let Some(lean) = q.get(16 + i) {
                        vs.wheels[i].camber = *lean;
                    }
                }
            }
        }
        // (MCLA_REPLAY_STATE: the whole of the recorded car before each tick - place, matrix,
        // velocities, each wheel's compression, spin, tyre displacements, slide, tyre torque,
        // compression rate and contact speed, the engine's and the shafts' speeds, the
        // differentials, gear and drive - so that every tick is one step from the original's
        // own state, and what differs after it is this port's rule, not its history)
        if let Some(q) = states.get(n).filter(|q| q.len() >= 61) {
            if seeded {
                // (what this port's own standing car holds otherwise)
                let mut differs = |name: &str, own: &[f32], rec: &[f32]| {
                    if own.iter().zip(rec).any(|(a, b)| a.to_bits() != b.to_bits() && !(*a == 0.0 && *b == 0.0)) {
                        println!("START differs: {name}: own {own:?} recorded {rec:?}");
                    }
                };
                let m = glam::Mat3::from_quat(c.rot);
                differs("place", &c.pos.to_array(), &q[0..3]);
                differs("matrix", &[m.x_axis.to_array(), m.y_axis.to_array(), m.z_axis.to_array()].concat(), &q[3..12]);
                differs("velocity", &c.vel.to_array(), &q[12..15]);
                differs("turning rate", &c.ang_vel.to_array(), &q[15..18]);
                if let Some(vs) = c.veh.as_ref() {
                    for i in 0..4 {
                        let w = &vs.wheels[i];
                        let r = &q[18 + i * 9..27 + i * 9];
                        differs(&format!("wheel {i} compression"), &[w.comp], &r[0..1]);
                        differs(&format!("wheel {i} spin"), &[w.spin_rate], &r[1..2]);
                        differs(&format!("wheel {i} tyre displacement (sideways, lengthways)"), &[w.disp_lat, w.disp_long], &r[2..4]);
                        differs(&format!("wheel {i} slide"), &[w.slide], &r[4..5]);
                        differs(&format!("wheel {i} tyre torque"), &[w.tyre_torque], &r[5..6]);
                        differs(&format!("wheel {i} compression rate"), &[w.comp_rate], &r[6..7]);
                        differs(&format!("wheel {i} contact speed along"), &[w.velocity_long], &r[7..8]);
                        differs(&format!("wheel {i} load"), &[w.load], &r[8..9]);
                        if q.len() >= 157 {
                            differs(&format!("wheel {i} lean"), &[w.camber], &[q[146 + i] * super::exact::sign(q[126 + 5 * i])]);
                            differs(&format!("wheel {i} slip along"), &[w.slip_long], &q[153 + i..154 + i]);
                        }
                    }
                    differs("engine speed", &[vs.engine_w], &q[54..55]);
                    differs("shaft speeds", &vs.omega, &q[55..57]);
                    differs("differentials", &vs.diff, &q[57..59]);
                    differs("gear", &[vs.gear as f32], &q[59..60]);
                    differs("drive layout", &[vs.drive as f32], &q[60..61]);
                    if q.len() >= 178 {
                        differs("boost time, torque multiplier, pedal", &[vs.boost_timer, vs.torque_mult, vs.engine_pedal], &q[157..160]);
                        differs("gyro: last error, drift level", &[vs.turn_err, vs.drift_level], &q[169..171]);
                        if let Some(b) = vs.body {
                            differs("body place", &b.at.to_array(), &q[150..153]);
                            differs("momenta", &[b.p.to_array(), b.l.to_array()].concat(), &q[171..177]);
                            differs("count of moves", &[b.count as f32], &q[177..178]);
                        } else {
                            println!("START differs: the body is not kept in the original's numbers (made from the car's quaternion each tick while it is held)");
                        }
                        differs("centre of mass height and length", &vs.cg, &[q[161], q[162]]);
                    }
                }
            }
            c.pos = Vec3::new(q[0], q[1], q[2]);
            c.rot = glam::Quat::from_mat3(&glam::Mat3::from_cols(Vec3::new(q[3], q[4], q[5]), Vec3::new(q[6], q[7], q[8]), Vec3::new(q[9], q[10], q[11]))).normalize();
            c.vel = Vec3::new(q[12], q[13], q[14]);
            c.ang_vel = Vec3::new(q[15], q[16], q[17]);
            if let Some(vs) = c.veh.as_mut() {
                for i in 0..4 {
                    let w = &q[18 + i * 9..27 + i * 9];
                    let ws = &mut vs.wheels[i];
                    // (MCLA_REPLAY_OWNROWS: the wheels' shapes stand at what this port's own
                    // last update left them, not at the recorded wheels' rows)
                    if std::env::var_os("MCLA_REPLAY_OWNROWS").is_none() || ws.world.is_none() || !states.get(n.wrapping_sub(1)).is_some_and(|q| q.len() >= 61) {
                        ws.part = None;
                    }
                    ws.comp = w[0];
                    ws.spin_rate = w[1];
                    ws.disp_lat = w[2];
                    ws.disp_long = w[3];
                    ws.slide = w[4];
                    ws.tyre_torque = w[5];
                    ws.comp_rate = w[6];
                    ws.velocity_long = w[7];
                    ws.load = w[8];
                }
                vs.engine_w = q[54];
                vs.omega = [q[55], q[56]];
                vs.diff = [q[57], q[58]];
                vs.gear = q[59] as i32;
                vs.drive = q[60] as u8;
for i in 0..4 {
                    if let Some(lean) = q.get(61 + i).filter(|_| std::env::var_os("MCLA_REPLAY_OWNCAMBER").is_none()) {
                        vs.wheels[i].camber = *lean;
                    }
                }
                vs.still_timer = 0.0;
                // (and, to the bit: the body's matrix, and each wheel's world matrix as the
                // recorded car's last update wrote it with the chassis's matrix of that update
                // - what the original's collision stood the wheels' shapes at, 0x8255CEA0)
                if q.len() >= 125 {
                    let v = |i: usize| Vec3::new(q[i], q[i + 1], q[i + 2]);
                    vs.body_exact = Some([v(3), v(6), v(9), v(0)]);
                    let then = [v(113), v(116), v(119), v(122)];
                    // (and the chassis's up of that update: what the wheels' contacts were weighed against, 0x8255D028)
                    vs.prev_up = Some(then[1]);
                    for i in 0..4 {
                        let o = 65 + 12 * i;
                        if vs.wheels[i].part.is_none() {
                            vs.wheels[i].part = Some(super::gjk::part_matrix(&[v(o), v(o + 3), v(o + 6), v(o + 9)], &then));
                        }
                    }
                    // (and the car's set-up as recorded: each wheel's place of rest, pivot arm
                    // and offset - wheel +384, +416, +432)
                    if q.len() >= 146 {
                        let mut r = [[0.0f32; 5]; 4];
                        for i in 0..4 {
                            r[i].copy_from_slice(&q[126 + 5 * i..131 + 5 * i]);
                        }
                        vs.rest_exact = Some(r);
                    }
                    // (and each wheel's lean as it stands, wheel +516: kept from tick to tick
                    // while the wheel is not pressed in past its offset)
                    if q.len() >= 150 {
                        for i in 0..4 {
                            vs.wheels[i].camber = q[146 + i] * super::exact::sign(q[126 + 5 * i]);
                        }
                    }
                    // (the body's own place, body +256, and each wheel's slip along its way as
                    // its last tyre update left it, wheel +560)
                    if q.len() >= 157 {
                        vs.body_at_exact = Some(v(150));
                        for i in 0..4 {
                            vs.wheels[i].slip_long = q[153 + i];
                        }
                    }
                    // (the engine's boost time left, its torque multiplier and its pedal as the
                    // last update left them: engine +28, +32, +24)
                    if q.len() >= 160 {
                        vs.boost_timer = q[157];
                        vs.torque_mult = q[158];
                        vs.engine_pedal = q[159];
                    }
                    // (the centre of mass as it stands, chassis +112; and, where
                    // MCLA_REPLAY_GYROSUMS asks for it, the recorded sums after the gyro)
                    if q.len() >= 169 {
                        vs.com_exact = Some(v(160));
                        vs.gyro_sums_exact = std::env::var_os("MCLA_REPLAY_GYROSUMS").map(|_| [v(163), v(166)]);
                    }
                    // (the gyro's own state: the turn's last error, gyro +68, and the drift
                    // thrust's level, gyro +56)
                    if q.len() >= 171 {
                        vs.turn_err = q[169];
                        vs.drift_level = q[170];
                    }
                    // (the body's momenta and its count of moves: body +304, +320, +664)
                    if q.len() >= 178 {
                        vs.momentum_exact = Some((v(171), v(174), q[177] as u32));
                    }
                    // ([196..202]: the push and turn the end of the recorded update gave the body,
                    // body +432 / +448, 0x8259E790. The sample's chassis block and body place are
                    // after them; the wheels' contacts the next update reads were found by the
                    // collision before them, at the pose the port keeps as `BodyExact::seen` - against
                    // a wall, deep pushes in a row, the front wheels 2.75 mm deeper than from the
                    // sample's pose (V work/carstep, s_rtv k 558). So the wheels are stood at the
                    // pose before the push: the push taken off, the turn turned back. Taking the push
                    // off a place 800 m out is not exact to the bit (a unit there is 6e-5 m: s_rtv s258,
                    // a rear wheel 3.9e-5 m deeper than the original's) - so where the port's own last
                    // tick left the body exactly where the sample has it, the pose its own collision
                    // saw before the push is the one.)
                    if q.len() >= 202 {
                        let (push, turn) = (v(196), v(199));
                        let own = vs.body.filter(|b| Some([b.rows[0], b.rows[1], b.rows[2], b.origin]) == vs.body_exact && Some(b.at) == vs.body_at_exact);
                        if let (Some(b), true) = (own, push != Vec3::ZERO || super::solver::turn_applies(turn)) {
                            vs.seen_exact = Some(b.seen);
                        } else if let (Some(b), Some(at), true) = (vs.body_exact, vs.body_at_exact, push != Vec3::ZERO || super::solver::turn_applies(turn)) {
                            let at = at - push;
                            let rows = super::solver::rows_pushed(&[b[0], b[1], b[2]], Vec3::ZERO - turn);
                            vs.seen_exact = Some([rows[0], rows[1], rows[2], super::exact::origin_of(at, &rows, pivot_local)]);
                        }
                    }
                    // (Z3, [178..196]: a motorcycle's gyro - +16, +20, +24, +36, +44, +48, +64..+72, +80..+88,
                    // +96, +100, +108, +112, +116, +120)
                    if q.len() >= 196 {
                        vs.bg = super::vehsim::BikeGyro { intent: 0, state: q[178] as i16, roll: q[179], roll_err: q[180], level: q[181], yaw_target: q[182], yaw_err: q[183], vel: v(184), acc: v(187), share: q[190], lat: q[191], boost: q[192], pitch: q[193], pitch_err: q[194], burn: q[195] };
                    }
                }
            }
        }
        for (i, a) in super::vehsim::REPLAY_SURFACE.iter().enumerate() {
            a.store(v[4 + i].to_bits(), std::sync::atomic::Ordering::Relaxed);
            super::vehsim::REPLAY_BUMP[i].store(v[8 + i].to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
        super::vehsim::REPLAY_WAY.store(v[12] as i32, std::sync::atomic::Ordering::Relaxed);
        // (this port's pedals: in reverse the brake drives)
        // (a sixteenth: the nitrous button)
        // (RB3: 1 = a hint, the shot is to be burning as this update begins; 2 = the logged pad: the frame
        // after this update has the button down - the fire rule decides)
        let nitro = v[15] > 0.5 && v[15] < 1.5;
        super::vehsim::REPLAY_NITRO_PAD.store(v[15] > 1.5, std::sync::atomic::Ordering::Relaxed);
        super::vehsim::REPLAY_RIDER.store(v[16].to_bits(), std::sync::atomic::Ordering::Relaxed);
        super::vehsim::REPLAY_JOINTED.store(v[17] > 0.5, std::sync::atomic::Ordering::Relaxed);
        super::vehsim::REPLAY_OTHERS.store(v[18] as u32, std::sync::atomic::Ordering::Relaxed);
        super::vehsim::REPLAY_ORDER.store(v[20] as u32, std::sync::atomic::Ordering::Relaxed);
        super::vehsim::REPLAY_OTHERS_E.store(v[19].to_bits(), std::sync::atomic::Ordering::Relaxed);
        let rider = (v[13], v[14] > 0.5);
        let v = if v[12] < 0.0 { [0.0, v[1], v[0], v[2], v[3]] } else { [0.0, v[0], v[1], v[2], v[3]] };
        // (MCLA_REPLAY_HOLD: the recording was made on a race's grid - chassis +592 bit 0x2, N-VP-1)
        let grid_hold = std::env::var("MCLA_REPLAY_HOLD").ok().map_or(false, |v| v.parse::<usize>().map_or(true, |k| n < k));
        let mut input = CarInput { throttle: v[1], brake: v[2], steer: v[3], handbrake: v[4] > 0.5, lean: rider.0, xfer: rider.1, nitro, hold: grid_hold, ..Default::default() };
        // (a state line's 126th value: the steering the recorded update turned its wheels with;
        // a line of one value alone: that steering and nothing else - the car runs on by itself)
        if let Some(q) = states.get(n).filter(|q| q.len() >= 126) {
            input.steer = q[125];
        } else if let Some(q) = states.get(n).filter(|q| q.len() == 1 && states.iter().any(|s| s.len() >= 126)) {
            input.steer = q[0];
        }
        {
            c.step(&p, input, &col, dt);
            // (a recording begins with the original's car standing: its body is at rest to
            // the last bit - every velocity exactly nothing - which a car held by its tyres'
            // springs on a cambered road never quite is here. Until the first pedal the
            // replay's car is held as still.)
            static MOVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if input.throttle > 0.0 || input.brake > 0.0 || input.handbrake || input.steer != 0.0 {
                MOVING.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if !MOVING.load(std::sync::atomic::Ordering::Relaxed) && std::env::var_os("MCLA_REPLAY_NOSLEEP").is_none() && std::env::var_os("MCLA_REPLAY_STATE").is_none() && std::env::var_os("MCLA_REPLAY_POSES").is_none() && !states.get(n).is_some_and(|q| q.len() >= 61) {
                c.vel = Vec3::ZERO;
                c.ang_vel = Vec3::ZERO;
                // (exactly where the recording's stands: a frozen body is not at its
                // suspension's balance, it is where it was when it froze)
                if let Some((at, turned)) = start_pose.filter(|_| std::env::var_os("MCLA_REPLAY_NOHOLD").is_none()) {
                    // (MCLA_REPLAY_NUDGE=metres: the start moved that far along x - how much a run
                    // owes to its last digits)
                    c.pos = at + Vec3::new(std::env::var("MCLA_REPLAY_NUDGE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0), 0.0, 0.0);
                    c.rot = turned;
                    start = at;
                }
                // (and its tyres stand as the recording's stood: MCLA_REPLAY_DISP holds each
                // wheel's sideways and lengthways displacement at the recording's start)
                if let (Some(d), Some(vs)) = (std::env::var("MCLA_REPLAY_DISP").ok().map(|d| d.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<f32>>()).filter(|d| d.len() >= 8), c.veh.as_mut()) {
                    for i in 0..4 {
                        vs.wheels[i].disp_lat = d[i * 2];
                        vs.wheels[i].disp_long = d[i * 2 + 1];
                    }
                }
            }
            // (MCLA_REPLAY_BITS=<file>: what the tick left, to the bit - one line of
            // hexadecimal float32 a tick: each wheel's compression, its rate, load, tyre force
            // sideways and lengthways, tyre displacements, spin; the force and torque sums;
            // velocity, angular velocity, place, the matrix's rows; engine and shaft speeds.
            // For laying beside a recording's next sample, work/veh/gjkfix/bitscore.py.)
            {
                static BITS: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
                if let Some(file) = BITS.get_or_init(|| std::env::var_os("MCLA_REPLAY_BITS").and_then(|p| std::fs::File::create(p).ok()).map(std::sync::Mutex::new)) {
                    let vs = c.veh.as_ref().unwrap();
                    let mut v: Vec<f32> = Vec::with_capacity(64);
                    for w in &vs.wheels {
                        v.extend([w.comp, w.comp_rate, w.load, w.f_lat, w.f_long, w.disp_lat, w.disp_long, w.spin_rate]);
                    }
                    let m = vs.body_now.unwrap_or_else(|| {
                        let m = glam::Mat3::from_quat(c.rot);
                        [m.x_axis, m.y_axis, m.z_axis, c.pos]
                    });
                    for p in [vs.last_force, vs.last_torque, c.vel, c.ang_vel, m[3], m[0], m[1], m[2]] {
                        v.extend(p.to_array());
                    }
                    v.extend([vs.engine_w, vs.omega[0], vs.omega[1]]);
                    // (59..107: each wheel's own matrix in the world, rows and place)
                    for w in &vs.wheels {
                        for r in w.world.unwrap_or([Vec3::ZERO; 4]) {
                            v.extend(r.to_array());
                        }
                    }
                    // (107..139: the contact record each wheel worked with - point, normal -
                    // and the contact's speed sideways and along the wheel)
                    for w in &vs.wheels {
                        v.extend(w.contact_in[0].to_array());
                        v.extend(w.contact_in[1].to_array());
                        v.extend([w.velocity_lat, w.velocity_long]);
                    }
                    // (139..143: the engine's torque, multiplier, boost time and pedal)
                    v.extend([vs.engine_torque, vs.torque_mult, vs.boost_timer, vs.engine_pedal]);
                    // (143..155: the force and torque sums before the gyro and after it)
                    for s in vs.sums_gyro {
                        v.extend(s.to_array());
                    }
                    // (155..164: the body's momenta and its own place)
                    if let Some(b) = vs.body {
                        v.extend(b.p.to_array());
                        v.extend(b.l.to_array());
                        v.extend(b.at.to_array());
                    } else {
                        v.extend([0.0; 9]);
                    }
                    // (164..168: each wheel's damping value)
                    v.extend(vs.wheels.iter().map(|w| w.damp_value));
                    // (168..176: each wheel's slide value and tyre torque)
                    v.extend(vs.wheels.iter().map(|w| w.slide));
                    v.extend(vs.wheels.iter().map(|w| w.tyre_torque));
                    // (176..194: a motorcycle's gyro state, as [178..196] of a state line)
                    let g = &vs.bg;
                    v.extend([g.state as f32, g.roll, g.roll_err, g.level, g.yaw_target, g.yaw_err, g.vel.x, g.vel.y, g.vel.z, g.acc.x, g.acc.y, g.acc.z, g.share, g.lat, g.boost, g.pitch, g.pitch_err, g.burn]);
                    let line: Vec<String> = v.iter().map(|x| format!("{:08x}", x.to_bits())).collect();
                    let _ = writeln!(file.lock().unwrap(), "{}", line.join(" "));
                }
            }
            t += dt;
            // (the speed a recording shows: the way made in the tick along the car's length)
            let moved = (c.pos - before).dot(c.forward()) / dt;
            before = c.pos;
            let vs = c.veh.as_ref().unwrap();
            let w = &vs.wheels;
            let flat = Vec3::new(c.pos.x - start.x, 0.0, c.pos.z - start.z).length();
            writeln!(
                out,
                "{:.4},{},{},{},{:.6},{:.6},{:.4},{:.1},{},{:.4},{:.3},{:.3},{:.4},{:.3},{:.3},{:.3},{:.3},{:.0},{:.0},{:.0},{:.0},{:.4},{:.4},{:.0},{:.0},{:.4},{:.4},{:.3},{:.1},{},{:.4},{:.3},{:.4},{:.5},{:.0},{:.0},{:.4},{:.4},{:.0},{:.0},{:.0},{:.0},{:.5},{:.1},{:.0},{:.0},{:.5},{:.5},{:.5},{:.0},{:.0},{:.0},{:.0},{:.0},{:.0},{:.0},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.3},{:.3},{:.3},{:.3},{:.3},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.5},{:.5},{:.5},{:.6},{:.6},{:.6},{:.6}",
                t, v[1], v[2], v[3], flat, c.pos.y - start.y, c.fwd_speed(), vs.engine_w * 9.549_296, vs.gear, vs.shift_timer, vs.omega[0], vs.omega[1], vs.diff[0],
                w[0].spin_rate, w[1].spin_rate, w[2].spin_rate, w[3].spin_rate, w[0].load, w[1].load, w[2].load, w[3].load, w[0].comp, w[2].comp, w[0].f_long, w[2].f_long,
                w[0].slip_long, w[2].slip_long, vs.engine_pedal, vs.engine_torque, vs.drive, vs.melt, vs.boost_timer, moved, c.forward().x.atan2(-c.forward().z), w[0].f_lat, w[2].f_lat, vs.steer, c.vel.dot(c.rot * Vec3::X), w[1].f_lat, w[3].f_lat, w[1].f_long, w[3].f_long, c.ang_vel.dot(c.up()), vs.turn_force, w[1].load, w[3].load, (c.rot.inverse() * c.ang_vel).x, (c.rot.inverse() * c.ang_vel).y, (c.rot.inverse() * c.ang_vel).z, vs.last_force.x, vs.last_force.y, vs.last_force.z, vs.last_torque.x, vs.last_torque.y, vs.last_torque.z, vs.last_gyro.y, c.ang_vel.y, w[0].slip_lat, w[2].slip_lat, w[0].disp_lat, w[2].disp_lat, w[0].disp_long, w[2].disp_long, w[0].slide, w[2].slide, (c.rot * Vec3::X).y.clamp(-1.0, 1.0).asin().to_degrees(), vs.lean.to_degrees(), c.forward().y.clamp(-1.0, 1.0).asin().to_degrees(), vs.wheelie, c.vel.x, c.vel.y, c.vel.z, c.ang_vel.x, c.ang_vel.y, c.ang_vel.z, w[1].comp, w[3].comp, w[1].disp_lat, w[3].disp_lat, w[1].disp_long, w[3].disp_long, c.pos.x, c.pos.y, c.pos.z, w[0].camber, w[1].camber, w[2].camber, w[3].camber
            )?;
        }
    }
    println!("replay: {} ticks -> {}", (t * 60.0).round(), f[3]);
    Ok(())
}
