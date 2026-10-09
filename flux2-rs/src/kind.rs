//! Die vier Illustrationsarten der Skripte (`token.sh`, `location.sh`,
//! `portrait.sh`, `fullbody.sh`) als Rust-Daten: Stiltext, Vorgabegrößen und
//! der Aufbau des Prompts.
//!
//! Die Stiltexte stehen hier wortgleich wie in den Skripten. Ändern heißt, die
//! Reihe neu zu beginnen — deshalb werden sie nicht pro Auftrag überschrieben,
//! außer ausdrücklich über `style`.
//!
//! Verneinungen fehlen absichtlich: klein ist distilliert (`cfg_scale 1.0`),
//! es gibt keinen Negativ-Prompt, und eine Verneinung im Positiv-Prompt kodiert
//! nur das Wort, das sie ausschließen soll.

use serde::{Deserialize, Serialize};

/// Welche Skriptfamilie ein Auftrag nachbaut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Freigestelltes Figuren-Token auf Sockel (`token.sh`).
    Token,
    /// Ortsillustration im Querformat (`location.sh`).
    Location,
    /// Kopf-und-Schultern-Porträt (`portrait.sh`).
    Portrait,
    /// Stehende Figur von Kopf bis Fuß in einer Szene (`fullbody.sh`).
    Fullbody,
}

pub const TOKEN_STYLE: &str = "hand-painted fantasy character illustration, clean dark-brown ink outlines, \
soft painterly cel shading, matte finish, warm desaturated medieval colors, \
flat even ambient lighting, isolated asset, high angle view looking down on the \
figure from above, tabletop miniature standing on a pale elliptical sandstone base \
seen from above, full body, grounded adult proportions about seven and a half heads \
tall, head small relative to the body, mature adult face with defined cheekbones and \
jaw, solid mid-grey background";

/// Pose des Tokens: der Kopf dreht sich mit dem Körper, die Füße werden
/// ausdrücklich benannt — dort bricht die Anatomie zuerst.
pub const TOKEN_POSE: &str = "calm upright pose, body turned three-quarters to the left, \
head facing the same way as the body, standing evenly on both feet, \
both boots flat on the base and pointing the same way as the body, \
arms and equipment close to body";

pub const LOCATION_STYLE: &str = "small rectangular fantasy location illustration, hand-drawn pen and ink \
engraving, fine dark sepia outlines, subtle watercolor wash, warm aged \
parchment paper, muted beige and brown palette, medieval fantasy travel \
journal illustration, architectural line drawing, fine cross-hatching, \
slightly imperfect hand-drawn lines, flat frontal composition, wide \
landscape view";

pub const PORTRAIT_STYLE: &str = "D&D fantasy character portrait, head and shoulders, \
three-quarter view, face clearly visible, tight close-up crop filling almost \
the entire frame, subject cropped at the shoulders and just above the top of \
the head, very little background visible around the figure, centered \
composition, hand-painted medieval fantasy illustration, fine dark brown ink \
outlines, soft painterly cel shading, subtle watercolor and parchment \
texture, natural facial features, expressive eyes, realistic proportions, \
detailed hair and clothing, restrained medieval fantasy design, warm earthy \
colors, muted brown green ochre and cream palette, soft warm light from the \
upper left, gentle shadows around the face, plain warm beige parchment \
background";

pub const FULLBODY_STYLE: &str = "full-body fantasy character illustration, vertical portrait \
orientation, entire figure shown from head to toe filling the frame, \
natural dynamic pose, painterly semi-realistic concept art in the style of \
modern fantasy game splash art, soft visible brushwork, expressive face with \
large detailed eyes, worn practical clothing with believable fabric folds, \
muted desaturated palette of cool blue-grey tones with warm brown accents, \
soft diffused daylight, gentle atmospheric perspective, softly blurred \
background scene with pale hazy depth and small figures far behind, shallow \
depth of field, subtle dust in the air, polished professional illustration";

/// Anweisung vor dem Prompt, wenn ein Referenzbild nur den Stil liefern soll.
/// Ohne sie übernimmt FLUX.2 bei einer Referenz Figur, Pose und Komposition.
pub const STYLE_REF_HINT: &str = "Use the reference image only as a style guide for its painterly \
brushwork, color palette, lighting and atmospheric depth. Create a completely \
new character and scene, with a different face, figure, pose, clothing and setting.";

/// Anweisung vor dem Prompt, wenn ein Referenzbild als **Szene** dient (`refs`,
/// "als Inhalt übernehmen"). FLUX.2 muss ausdrücklich erfahren, wozu eine
/// Referenz da ist; ohne den Satz wird sie als lose Anregung behandelt.
pub const REF_SZENE_HINT: &str = "Use the reference image as the setting: keep its location, \
architecture, perspective, lighting and colors exactly as shown, and place the following \
character naturally into that scene.";

/// Der Ganzkörper-Stil **ohne** alles, was die Umgebung beschreibt (Hintergrund,
/// Palette, Licht, Tiefe, Staub) und ohne "filling the frame". Mit einer
/// Szenenreferenz würde der normale Stil gegen das Bild arbeiten: er verlangt
/// einen unscharfen Hintergrund mit kleinen Figuren und kühle Blaugrau-Töne.
pub const FULLBODY_STYLE_SZENE: &str = "full-body fantasy character illustration, vertical portrait \
orientation, entire figure shown from head to toe, natural dynamic pose, painterly \
semi-realistic concept art in the style of modern fantasy game splash art, soft visible \
brushwork, expressive face with large detailed eyes, worn practical clothing with believable \
fabric folds, polished professional illustration";

/// Wie ein Freistellen der Token-Variante aussieht: auf das Motiv zuschneiden
/// und in diese Fläche einpassen.
pub const TOKEN_OUT_W: u32 = 138;
pub const TOKEN_OUT_H: u32 = 244;

/// Größe und Schrittzahl, die ein Skript ohne weitere Angaben benutzt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vorgaben {
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    /// `true`: Hintergrund bleibt Teil des Bildes (kein Freistellen).
    pub keep_bg: bool,
}

impl Kind {
    pub const ALLE: [Kind; 4] = [Kind::Token, Kind::Location, Kind::Portrait, Kind::Fullbody];

    /// Der Skriptname, den die Listen-Schnittstelle und die Doku benutzen.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Token => "token",
            Kind::Location => "location",
            Kind::Portrait => "portrait",
            Kind::Fullbody => "fullbody",
        }
    }

    /// Vorgaben aus den Skripten. `gross` ist `location.sh --large`
    /// (1536x768 statt 1024x512); bei den anderen Arten ohne Wirkung.
    pub fn vorgaben(self, gross: bool) -> Vorgaben {
        match self {
            Kind::Token => Vorgaben { width: 1024, height: 1024, steps: 8, keep_bg: false },
            Kind::Location if gross => {
                Vorgaben { width: 1536, height: 768, steps: 8, keep_bg: true }
            }
            Kind::Location => Vorgaben { width: 1024, height: 512, steps: 8, keep_bg: true },
            Kind::Portrait => Vorgaben { width: 1024, height: 1024, steps: 8, keep_bg: true },
            Kind::Fullbody => Vorgaben { width: 768, height: 1536, steps: 8, keep_bg: true },
        }
    }

    /// Der feste Stiltext dieser Art.
    pub fn stil(self) -> &'static str {
        match self {
            Kind::Token => TOKEN_STYLE,
            Kind::Location => LOCATION_STYLE,
            Kind::Portrait => PORTRAIT_STYLE,
            Kind::Fullbody => FULLBODY_STYLE,
        }
    }

    /// Setzt den Prompt zusammen wie das jeweilige Skript.
    ///
    /// `style` ersetzt den Stiltext (nur für Experimente — sonst bricht die
    /// Reihe), `pose` ersetzt die Pose und wirkt nur bei Tokens.
    pub fn prompt(self, beschreibung: &str, style: Option<&str>, pose: Option<&str>) -> String {
        let stil = style.unwrap_or_else(|| self.stil());
        match self {
            Kind::Token => {
                let pose = pose.unwrap_or(TOKEN_POSE);
                format!("{beschreibung}, {pose}, {stil}")
            }
            _ => format!("{beschreibung}, {stil}"),
        }
    }
}

impl Kind {
    /// Wie `prompt`, aber für einen Auftrag mit Szenenreferenz: bei Ganzkörper
    /// fällt der Umgebungsteil des Stils weg, damit er dem Bild nicht widerspricht.
    /// Die anderen Arten behalten ihren Stil.
    pub fn prompt_mit_szene(self, beschreibung: &str, style: Option<&str>, pose: Option<&str>) -> String {
        let stil = match (self, style) {
            (Kind::Fullbody, None) => Some(FULLBODY_STYLE_SZENE),
            _ => style,
        };
        format!("{REF_SZENE_HINT} {}", self.prompt(beschreibung, stil, pose))
    }
}

impl std::str::FromStr for Kind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Kind::ALLE
            .into_iter()
            .find(|k| k.name() == s.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                format!(
                    "unbekannte Art '{s}' — gültig: {}",
                    Kind::ALLE.map(|k| k.name()).join(", ")
                )
            })
    }
}

/// Dateiname-Stamm aus dem ersten Teil der Beschreibung, wie die Skripte ihn
/// bilden: alles bis zum ersten Komma, klein geschrieben, jede Folge von
/// Nicht-Buchstaben/-Ziffern wird zu einem `-`, Ränder werden beschnitten.
///
/// `None`, wenn nichts übrig bleibt — dann muss der Aufrufer einen Namen geben.
pub fn slug(beschreibung: &str) -> Option<String> {
    let erster_teil = beschreibung.split(',').next().unwrap_or("");
    let mut slug = String::new();
    for zeichen in erster_teil.chars() {
        if zeichen.is_ascii_alphanumeric() {
            slug.push(zeichen.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    (!slug.is_empty()).then(|| slug.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_prompt_enthaelt_pose_vor_dem_stil() {
        let prompt = Kind::Token.prompt("female human warrior", None, None);
        assert_eq!(prompt, format!("female human warrior, {TOKEN_POSE}, {TOKEN_STYLE}"));
    }

    #[test]
    fn andere_arten_haengen_nur_den_stil_an() {
        assert_eq!(
            Kind::Location.prompt("a small village", None, None),
            format!("a small village, {LOCATION_STYLE}")
        );
        assert_eq!(
            Kind::Portrait.prompt("elf ranger", None, None),
            format!("elf ranger, {PORTRAIT_STYLE}")
        );
        assert_eq!(
            Kind::Fullbody.prompt("orphan girl", None, None),
            format!("orphan girl, {FULLBODY_STYLE}")
        );
    }

    #[test]
    fn style_ersetzt_den_stiltext() {
        let prompt = Kind::Fullbody.prompt("orphan girl", Some("flat vector art"), None);
        assert_eq!(prompt, "orphan girl, flat vector art");
    }

    #[test]
    fn pose_ersetzt_nur_bei_tokens() {
        let token = Kind::Token.prompt("dwarf", None, Some("arms crossed"));
        assert!(token.starts_with("dwarf, arms crossed, "), "{token}");
        assert!(!token.contains("calm upright pose"));
        let ort = Kind::Location.prompt("village", None, Some("arms crossed"));
        assert!(!ort.contains("arms crossed"), "Pose gehört nur zu Tokens: {ort}");
    }

    #[test]
    fn vorgaben_entsprechen_den_skripten() {
        let token = Kind::Token.vorgaben(false);
        assert_eq!((token.width, token.height, token.steps), (1024, 1024, 8));
        assert!(!token.keep_bg, "Tokens werden freigestellt");

        let fullbody = Kind::Fullbody.vorgaben(false);
        assert_eq!((fullbody.width, fullbody.height), (768, 1536));
        assert!(fullbody.keep_bg);

        let ort = Kind::Location.vorgaben(false);
        assert_eq!((ort.width, ort.height), (1024, 512));
        let gross = Kind::Location.vorgaben(true);
        assert_eq!((gross.width, gross.height), (1536, 768));
        assert_eq!(Kind::Portrait.vorgaben(true), Kind::Portrait.vorgaben(false));
    }

    #[test]
    fn alle_vorgabegroessen_sind_durch_16_teilbar() {
        for kind in Kind::ALLE {
            for gross in [false, true] {
                let v = kind.vorgaben(gross);
                assert_eq!((v.width % 16, v.height % 16), (0, 0), "{kind:?}");
            }
        }
    }

    #[test]
    fn stiltexte_enthalten_keine_verneinungen() {
        // Bei cfg_scale 1.0 kodiert "no X" im Positiv-Prompt nur das Wort X.
        for kind in Kind::ALLE {
            let stil = kind.stil().to_ascii_lowercase();
            for verbot in [" no ", "without ", "not "] {
                assert!(!stil.contains(verbot), "{kind:?} enthält '{verbot}'");
            }
        }
    }

    #[test]
    fn art_wird_aus_text_gelesen() {
        assert_eq!("fullbody".parse::<Kind>(), Ok(Kind::Fullbody));
        assert_eq!(" Token ".parse::<Kind>(), Ok(Kind::Token));
        let fehler = "comic".parse::<Kind>().unwrap_err();
        assert!(fehler.contains("comic") && fehler.contains("fullbody"), "{fehler}");
    }

    #[test]
    fn slug_nimmt_den_ersten_teil_bis_zum_komma() {
        assert_eq!(
            slug("sad young orphan girl standing barefoot, worn dress").as_deref(),
            Some("sad-young-orphan-girl-standing-barefoot")
        );
    }

    #[test]
    fn slug_faltet_sonderzeichen_und_beschneidet_raender() {
        assert_eq!(slug("  A  Tall -- Dwarf!! ").as_deref(), Some("a-tall-dwarf"));
        assert_eq!(slug("Half-Orc (Barbarian)").as_deref(), Some("half-orc-barbarian"));
    }

    #[test]
    fn slug_ohne_verwertbare_zeichen_ist_none() {
        assert_eq!(slug(""), None);
        assert_eq!(slug("!!!, text"), None);
    }

    #[test]
    fn szenenprompt_stellt_die_anweisung_voran_und_laesst_bei_ganzkoerper_die_umgebung_weg() {
        let p = Kind::Fullbody.prompt_mit_szene("orphan girl", None, None);
        assert!(p.starts_with(REF_SZENE_HINT), "{p}");
        assert!(p.contains("orphan girl, full-body fantasy character illustration"));
        for umgebung in ["background", "blue-grey", "daylight", "filling the frame", "dust"] {
            assert!(!p.contains(umgebung), "'{umgebung}' widerspräche der Szene: {p}");
        }
    }

    #[test]
    fn szenenprompt_behaelt_den_stil_anderer_arten_und_eigenen_style() {
        let p = Kind::Portrait.prompt_mit_szene("elf", None, None);
        assert!(p.ends_with(PORTRAIT_STYLE), "{p}");
        let p = Kind::Fullbody.prompt_mit_szene("girl", Some("flat vector art"), None);
        assert!(p.ends_with("girl, flat vector art"), "{p}");
    }

    #[test]
    fn szenenstil_ist_der_ganzkoerperstil_ohne_umgebungsteil() {
        // Jede Wendung des Szenenstils muss im normalen Stil vorkommen — er ist
        // eine Kürzung, keine zweite Stilrichtung.
        for teil in FULLBODY_STYLE_SZENE.split(", ") {
            assert!(FULLBODY_STYLE.contains(teil) || teil == "entire figure shown from head to toe", "{teil}");
        }
    }

    #[test]
    fn style_ref_hinweis_sagt_positiv_was_neu_entsteht() {
        assert!(STYLE_REF_HINT.contains("only as a style guide"));
        assert!(STYLE_REF_HINT.contains("completely new character"));
    }
}
