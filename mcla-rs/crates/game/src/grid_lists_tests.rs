//! The object grid's cell lists (`GridLists`) against the original's insert / move F-66511 and unlink F-60332 run offline
//! (tools/object_grid_reference.py): 300 grids (some 300 x 300, past 16-bit cell numbers), 60 filings and unlinks of 8
//! objects each, on cell borders, in cell 0, off the grid; every cell's list from its head after the last.

use super::*;

#[test]
fn grid_lists_match_the_original() {
    let f = |w: &str| f32::from_bits(u32::from_str_radix(w, 16).unwrap());
    let (mut grid, mut lists) = (ObjectGrid::unbounded(), GridLists::default());
    let (mut cases, mut linked) = (0, 0);
    for line in include_str!("testdata/object_grid_ref.txt").lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w[0] {
            "G" => {
                let (nx, nz) = (w[1].parse::<i32>().unwrap(), w[2].parse::<i32>().unwrap());
                grid = ObjectGrid { min: [f(w[3]), f(w[4])], n: [nx, nz], bounded: true };
                lists = GridLists::default();
            }
            "F" => lists.file(&grid, w[1].parse().unwrap(), Vec3::new(f(w[2]), 0.0, f(w[3]))),
            "U" => lists.unlink_in(&grid, w[1].parse().unwrap()),
            _ => {
                let mut got = String::from("->");
                for k in 0..grid.n[0] as i64 * grid.n[1] as i64 {
                    let l = lists.list(k);
                    if !l.is_empty() {
                        got += &format!(" {k}:{}", l.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","));
                        linked += 1;
                    }
                }
                assert_eq!(got, line, "case {cases}");
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 300);
    assert!(linked > 500, "{linked}");
}
