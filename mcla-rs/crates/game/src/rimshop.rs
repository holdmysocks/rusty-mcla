//! The garage's rim shop, as the disc's own menu scripts define it.
//!
//! `tune/ui/garage/elements/Rim.sc.xml` (cars), `RimBike.sc.xml` (bikes) and
//! `DLC_Rim.sc.xml` (the `whld_am_*` downloadable rims) each hold one `<state
//! id="rim_...">` per wheel brand. A brand carries `fs_mfr`, the maker id that
//! selects the `vs_brands` logo the movie shows at frame `mfr + 1`, and the tab
//! it takes in the rim panel (`nav.setFlashVar('cur_tab', n)`). Each of its
//! child states is one rim: `fs_text1` names the resource folder under
//! `resources/rims`, `fs_text2` (repeated as `price`) is what it costs, and
//! `index` is the game's own rim-table index.
//!
//! That index is *not* the order the port lists rims in - it runs 39..174 over
//! the disc's 101 buyable folders and skips the unlisted ones - so a rim is
//! always resolved by its resource name, never by position.
//!
//! Every brand uses `fs_template` 47, the movie's `rimselection` panel: a short
//! list of `fs_List_max` rims, the brand logo, and `btn_lb` / `btn_rb` to step
//! between brands.

/// Which of the shop's three files a brand came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shop {
    /// `Rim.sc.xml`: the rims any car may buy
    Car,
    /// `RimBike.sc.xml`: motorcycles only
    Bike,
    /// `DLC_Rim.sc.xml`: the `whld_am_*` downloadable rims, free (price 0)
    Dlc,
}

/// One rim of one brand.
#[derive(Clone, Debug)]
pub struct RimEntry {
    /// the resource folder under `resources/rims` (`fs_text1`)
    pub resource: String,
    /// what it costs (`fs_text2`, which equals `price`); 0 for the DLC rims
    pub price: i64,
    /// the game's own rim-table index (`index`)
    pub index: i32,
}

impl RimEntry {
    /// the name the shop lists the rim under: the resource folder with its
    /// `whld_am_` / `whl_am_` prefix off and its `_`s read as spaces.
    pub fn label(&self) -> String {
        let folder = folder_for(&self.resource);
        folder.strip_prefix("whld_am_").or_else(|| folder.strip_prefix("whl_am_")).unwrap_or(folder).replace('_', " ").to_uppercase()
    }
}

/// The one rim the disc's own `DLC_Rim.sc.xml` misspells: the folder under
/// `resources/rims` is `whld_am_dytn_rlwelllace` and the script says
/// `whld_am_dytn_rlwellace`. The game loads rims through its own table index
/// rather than by folder, so it never notices; the port resolves by name and
/// would otherwise drop the rim from the shop.
const MISSPELLED: &[(&str, &str)] = &[("whld_am_dytn_rlwellace", "whld_am_dytn_rlwelllace")];

/// The resource folder a rim is really in, correcting the disc's own typos.
pub fn folder_for(resource: &str) -> &str {
    MISSPELLED.iter().find(|(wrong, _)| *wrong == resource).map_or(resource, |(_, right)| *right)
}

/// One wheel brand: a tab of the rim shop.
#[derive(Clone, Debug)]
pub struct RimBrand {
    /// the script state's id, e.g. `rim_bbs`
    pub id: String,
    /// `fs_mfr`: the maker id behind the `vs_brands` logo at frame `mfr + 1`
    pub mfr: i32,
    /// `cur_tab`: this brand's tab in the rim panel
    pub tab: i32,
    /// which file declared it
    pub shop: Shop,
    pub rims: Vec<RimEntry>,
}

/// The whole rim shop: every brand the disc offers, cars and bikes alike.
#[derive(Clone, Debug, Default)]
pub struct RimShop {
    /// every brand, in the order its file declares them
    pub brands: Vec<RimBrand>,
}

impl RimShop {
    /// The tabs a car of this kind may open: a motorcycle sees the bike brands
    /// too, everything else the car's own and the downloadable rims.
    pub fn tabs(&self, bike: bool) -> Vec<&RimBrand> {
        self.brands.iter().filter(|b| bike || b.shop != Shop::Bike).collect()
    }

    /// The rim the game's rim-table `index` names.
    pub fn entry(&self, index: i32) -> Option<&RimEntry> {
        self.brands.iter().flat_map(|b| &b.rims).find(|r| r.index == index)
    }

    /// The car rims (Car and Dlc brands, not Bike) in the file order, as (game rim-table index, resource folder): the
    /// rims a mod car's random opponent rim is drawn from (stage S4b).
    pub fn car_rims(&self) -> Vec<(i32, &str)> {
        self.brands.iter().filter(|b| b.shop != Shop::Bike).flat_map(|b| b.rims.iter().map(|r| (r.index, r.resource.as_str()))).collect()
    }

    /// The brand tab that sells a given rim, and its place in `tabs`.
    pub fn tab_of(&self, index: i32, bike: bool) -> Option<(usize, &RimBrand)> {
        self.tabs(bike).into_iter().enumerate().find(|(_, b)| b.rims.iter().any(|r| r.index == index))
    }

    /// The longest rim name in the whole shop, in characters.
    ///
    /// The garage movie sizes its panel by measuring the rows it is handed, so a
    /// brand with short names (`BBS CH`) draws a narrower panel than one with
    /// long names (`KONIG FF GRAPHITE`) and the shop visibly resizes as the tabs
    /// are stepped. Padding every row out to this length makes each tab measure
    /// the same, and the panel holds still.
    pub fn label_pad(&self) -> usize {
        self.brands.iter().flat_map(|b| &b.rims).map(|r| r.label().chars().count()).max().unwrap_or(0)
    }
}

/// Read the three menu scripts that make up the rim shop.
pub fn load(data_dir: &str) -> RimShop {
    let dir = std::path::Path::new(data_dir).join("tune/ui/garage/elements");
    let mut brands = Vec::new();
    for (file, shop) in [("Rim.sc.xml", Shop::Car), ("RimBike.sc.xml", Shop::Bike), ("DLC_Rim.sc.xml", Shop::Dlc)] {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else { continue };
        brands.extend(parse(&text, shop));
    }
    RimShop { brands }
}

/// Pick out the `<state id="rim_...">` blocks and the `data` values inside.
///
/// The files are plain text with one `<data>` per line and only the brand
/// states carry an id, so splitting on `<state id="rim_` is enough to separate
/// them and the `data` values can be read tag by tag. Anything the disc does
/// not put in the shape read here is skipped rather than guessed at.
fn parse(text: &str, shop: Shop) -> Vec<RimBrand> {
    let mut brands = Vec::new();
    for chunk in text.split("<state id=\"rim_").skip(1) {
        let Some((id, body)) = chunk.split_once('"') else { continue };
        let Some(mfr) = expr(body, "fs_mfr").and_then(|v| v.parse().ok()) else { continue };
        let rims = parse_rims(body);
        if rims.is_empty() {
            continue;
        }
        // `nav.setFlashVar('cur_tab',n)`; the position in the file is the fallback
        let tab = call_arg(body, "cur_tab").and_then(|v| v.parse().ok()).unwrap_or(brands.len() as i32 + 1);
        brands.push(RimBrand { id: id.to_string(), mfr, tab, shop, rims });
    }
    brands
}

/// The rims of one brand: each `fs_text1` with the `fs_text2` and `index` beside
/// it. A rim without an index is not one the game can fit, so it is dropped.
fn parse_rims(body: &str) -> Vec<RimEntry> {
    let mut rims = Vec::new();
    for row in body.split("<data name=\"fs_text1\"").skip(1) {
        let Some(resource) = expr(row, "") else { continue };
        let resource = resource.trim_matches('\'').to_string();
        if resource.is_empty() {
            continue;
        }
        let (Some(price), Some(index)) = (expr(row, "fs_text2").and_then(|v| v.parse().ok()), expr(row, "index").and_then(|v| v.parse().ok())) else { continue };
        rims.push(RimEntry { resource, price, index });
    }
    rims
}

/// The `expr` of the `<data name="<key>"` tag at the head of `text`.
fn expr(text: &str, key: &str) -> Option<String> {
    let head = if key.is_empty() { text } else { text.split_once(&format!("name=\"{}\"", key)).map_or(text, |(_, tail)| tail) };
    let at = head.find("expr=")?;
    let rest = head[at + 5..].trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].trim().to_string())
}

/// The numeric argument of `name(arg)`, e.g. `cur_tab'` in
/// `nav.setFlashVar('cur_tab',7)`.
fn call_arg(text: &str, name: &str) -> Option<String> {
    let at = text.find(&format!("{}',", name))?;
    let rest = text[at + name.len() + 2..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two brand states in the shape the disc's Rim.sc.xml has.
    const CAR: &str = r#"
   <state id="rim_konig">
    <data name="fs_template"	  expr="47"					        ></data>
    <data name="fs_iconID"		expr="10"					        ></data>
    <data name="fs_List_max"      expr="4"	                  ></data>
    <data name="fs_mfr" 	    expr="60"></data>
    <onenter expr="nav.setFlashVar('cur_tab',10)"></onenter>
    <state>
      <onenter  expr="garage.InstallRimOnVehicle()" ></onenter>
      <data name="fs_text1" expr="'whl_am_konig_ffgraphite'"></data>
      <data name="fs_text2" expr="832"></data>
      <data name="price" 	  expr="832"></data>
      <data name="index" 	  expr="57"></data>
    </state>
    <state>
      <data name="fs_text1" expr="'whl_am_konig_unknown'"></data>
      <data name="fs_text2" expr="1824"></data>
      <data name="price" 	  expr="1824"></data>
      <data name="index" 	  expr="62"></data>
    </state>
   </state>
   <state id="rim_bbs">
    <data name="fs_mfr" 	    expr="78"></data>
    <onenter expr="nav.setFlashVar('cur_tab',15)"></onenter>
    <state>
      <data name="fs_text1" expr="'whl_am_bbs_lm'"></data>
      <data name="fs_text2" expr="4560"></data>
      <data name="price" 	  expr="4560"></data>
      <data name="index" 	  expr="128"></data>
    </state>
   </state>
"#;

    #[test]
    fn reads_every_brand_tab_and_rim() {
        let brands = parse(CAR, Shop::Car);
        assert_eq!(brands.len(), 2, "both brand states");
        assert_eq!(brands[0].id, "konig", "the id is what follows rim_");
        assert_eq!(brands[0].mfr, 60);
        assert_eq!(brands[0].tab, 10);
        assert_eq!(brands[0].rims.len(), 2);
        assert_eq!(brands[0].rims[0].resource, "whl_am_konig_ffgraphite");
        assert_eq!(brands[0].rims[0].price, 832);
        assert_eq!(brands[0].rims[0].index, 57);
        // the second brand is a tab of its own, not a continuation of the first
        assert_eq!(brands[1].id, "bbs");
        assert_eq!(brands[1].mfr, 78);
        assert_eq!(brands[1].tab, 15);
        assert_eq!(brands[1].rims.len(), 1);
        assert_eq!(brands[1].rims[0].resource, "whl_am_bbs_lm");
        assert_eq!(brands[1].rims[0].price, 4560);
    }

    #[test]
    fn a_rim_without_a_price_or_index_is_not_offered() {
        let brands = parse(
            r#"<state id="rim_x"><data name="fs_mfr" expr="5"></data>
               <state><data name="fs_text1" expr="'whl_am_x_1'"></data>
               <data name="fs_text2" expr="10"></data></state></state>"#,
            Shop::Car,
        );
        assert!(brands.is_empty(), "a rim with no index cannot be fitted");
    }

    #[test]
    fn resolves_a_rim_by_the_games_index() {
        let shop = RimShop { brands: parse(CAR, Shop::Car) };
        assert_eq!(shop.entry(62).unwrap().resource, "whl_am_konig_unknown");
        assert_eq!(shop.entry(128).unwrap().price, 4560);
        assert!(shop.entry(999).is_none());
        assert_eq!(shop.tab_of(62, false).map(|(i, b)| (i, b.mfr)), Some((0, 60)));
        assert_eq!(shop.tab_of(128, false).map(|(i, b)| (i, b.mfr)), Some((1, 78)));
    }

    /// The disc spells one DLC rim's folder wrongly; the shop still offers it.
    #[test]
    fn a_rim_the_disc_misspells_still_resolves() {
        assert_eq!(folder_for("whld_am_dytn_rlwellace"), "whld_am_dytn_rlwelllace");
        // everything else is left alone
        assert_eq!(folder_for("whld_am_dytn_c2_crosswell"), "whld_am_dytn_c2_crosswell");
        assert_eq!(folder_for("whl_am_bbs_lm"), "whl_am_bbs_lm");
        let shop = RimShop {
            brands: parse(
                r#"<state id="rim_dayton"><data name="fs_mfr" expr="9"></data>
                   <state><data name="fs_text1" expr="'whld_am_dytn_rlwellace'"></data>
                   <data name="fs_text2" expr="0"></data><data name="index" expr="170"></data></state></state>"#,
                Shop::Dlc,
            ),
        };
        let rim = &shop.brands[0].rims[0];
        assert_eq!(rim.label(), "DYTN RLWELLLACE", "listed under the folder it is really in");
    }

    /// The disc's own shop: every brand, and a rim or two of each.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn the_discs_rim_shop_is_complete_and_priced() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let shop = load(&dir);
        assert!(shop.brands.len() >= 20, "brands found: {}", shop.brands.len());
        let rims: Vec<&RimEntry> = shop.brands.iter().flat_map(|b| &b.rims).collect();
        assert!(rims.len() >= 100, "rims found: {}", rims.len());
        // every maker id is a real logo frame in the movie's vs_brands (114 frames)
        for b in &shop.brands {
            assert!(b.mfr > 0 && b.mfr < 113, "{} mfr {}", b.id, b.mfr);
        }
        // Konig, as Rim.sc.xml declares it
        let konig = shop.brands.iter().find(|b| b.id == "konig").expect("konig");
        assert_eq!(konig.mfr, 60);
        assert_eq!(konig.rims.len(), 4);
        assert_eq!(konig.rims[0].resource, "whl_am_konig_ffgraphite");
        assert_eq!(konig.rims[0].price, 832);
        // the downloadable rims are free
        for b in shop.brands.iter().filter(|b| b.shop == Shop::Dlc) {
            for r in &b.rims {
                assert_eq!(r.price, 0, "{} {} should be free", b.id, r.resource);
                assert!(r.resource.starts_with("whld_am_"), "{}", r.resource);
            }
        }
        // no rim is listed twice
        let mut seen = std::collections::HashSet::new();
        for r in rims {
            assert!(seen.insert(r.index), "rim index {} listed twice", r.index);
        }
    }

    #[test]
    fn bikes_see_the_bike_brand_too() {
        let mut brands = parse(CAR, Shop::Car);
        brands.extend(parse(
            r#"<state id="rim_rc_bike"><data name="fs_mfr" expr="79"></data>
               <onenter expr="nav.setFlashVar('cur_tab',2)"></onenter>
               <state><data name="fs_text1" expr="'whl_am_rc_comp_vega'"></data>
               <data name="fs_text2" expr="2768"></data><data name="index" expr="139"></data></state></state>"#,
            Shop::Bike,
        ));
        let shop = RimShop { brands };
        assert_eq!(shop.tabs(false).len(), 2, "a car sees the car brands only");
        assert_eq!(shop.tabs(true).len(), 3, "a bike sees those and the bike brand");
    }
}