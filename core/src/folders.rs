//! Automatic folders for saves: keyword rules over what the post says, run on the device. No
//! network and no model, so it is free, instant and the same on every device. The user can
//! always move a save; a folder the user chose is never changed by these rules.

/// The folders every user starts with, in the order the apps show them. A save with no folder
/// is "Unsorted".
pub const FOLDERS: [&str; 9] =
    ["Watch Later", "Read Later", "Learn", "Recipes", "Fitness and Health", "Career and Money", "Buy Later", "Ideas", "Funny"];

/// Words and phrases per folder, lowercase. Single words match whole words (and hashtags);
/// phrases match anywhere. English plus the Hinglish that Indian posts use.
const RULES: &[(&str, &[&str])] = &[
    ("Recipes", &[
        "recipe", "recipes", "ingredients", "cook", "cooking", "bake", "baking", "dal", "paneer", "masala",
        "biryani", "curry", "roti", "paratha", "chutney", "dessert", "breakfast", "lunch", "dinner", "snack",
        "khana", "rasoi", "tadka", "sabzi", "chicken", "pasta", "cake", "cookies", "foodie", "airfryer",
        "cup of", "tbsp", "tsp", "minutes on", "pressure cooker",
    ]),
    ("Fitness and Health", &[
        "workout", "workouts", "gym", "exercise", "exercises", "fitness", "diet", "protein", "yoga", "abs",
        "cardio", "calories", "fatloss", "weightloss", "health", "healthy", "sleep", "stretch", "stretching",
        "skincare", "pcos", "posture", "squat", "squats", "pushups", "fat loss", "weight loss", "lose weight",
    ]),
    ("Career and Money", &[
        "job", "jobs", "career", "interview", "resume", "salary", "hiring", "invest", "investing", "investment",
        "stock", "stocks", "sip", "finance", "money", "income", "startup", "startups", "founder", "business",
        "tax", "taxes", "crypto", "trading", "budget", "savings", "linkedin", "freelance", "freelancing",
        "mutual fund", "side hustle", "passive income", "personal finance",
    ]),
    ("Buy Later", &[
        "buy", "price", "sale", "discount", "deal", "deals", "amazon", "flipkart", "myntra", "meesho", "shop",
        "shopping", "order", "unboxing", "product", "products", "gadget", "gadgets", "₹", "rs.", "under rs",
        "link in bio", "shop now", "order now", "use code", "coupon",
    ]),
    ("Learn", &[
        "learn", "learning", "tutorial", "course", "lesson", "guide", "explained", "explainer", "study",
        "coding", "code", "python", "javascript", "programming", "developer", "ai", "chatgpt", "claude", "llm",
        "excel", "english", "science", "history", "chess", "tips", "tricks", "how to", "step by step",
        "did you know", "open sourced", "design system",
    ]),
    ("Ideas", &[
        "idea", "ideas", "inspiration", "inspo", "diy", "design", "aesthetic", "decor", "hack", "hacks",
        "travel", "trip", "itinerary", "places", "gift", "gifts", "craft", "outfit", "outfits",
        "bucket list", "things to do",
    ]),
    ("Funny", &[
        "funny", "meme", "memes", "lol", "lmao", "comedy", "prank", "joke", "jokes", "😂", "🤣", "😆", "relatable",
    ]),
];

/// The folder these words point at, or a default by platform: videos to Watch Later, articles
/// to Read Later. None leaves the save Unsorted.
pub fn suggest_folder(platform: &str, words: &[&str]) -> Option<&'static str> {
    let text = words.join("\n").to_lowercase();
    let tokens: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '₹' || c == '.'))
        .map(|t| t.trim_matches('.'))
        .filter(|t| !t.is_empty())
        .collect();
    let mut best: Option<(&str, usize)> = None;
    for (folder, keys) in RULES {
        let hits = keys
            .iter()
            .filter(|k| {
                if k.contains(' ') || !k.chars().all(char::is_alphanumeric) {
                    text.contains(*k)
                } else {
                    tokens.contains(k)
                }
            })
            .count();
        // Earlier folders win ties: a recipe video is a recipe before it is something to learn.
        if hits > 0 && best.is_none_or(|(_, b)| hits > b) {
            best = Some((folder, hits));
        }
    }
    best.map(|(f, _)| f).or(match platform {
        "youtube" | "instagram" => Some("Watch Later"),
        "web" => Some("Read Later"),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_saves_land_somewhere_sensible() {
        let cases: &[(&str, &[&str], Option<&str>)] = &[
            ("instagram", &["#chess #chessplayer #explore #reels #fyp"], Some("Learn")),
            ("threads", &["Meta just open sourced the design system behind over 13K of its apps."], Some("Learn")),
            ("youtube", &["Finally Karma Caught Up to Abhi & Niyu? 😨"], Some("Watch Later")),
            ("x", &["Can I ask a dumb question? If money doesn't buy happiness, why are we sacrificing our best years trying to get rich?"], Some("Career and Money")),
            ("instagram", &["5 minute dal recipe, no onion. Ingredients: 1 cup of moong dal"], Some("Recipes")),
            ("instagram", &["30 day workout for fat loss at home"], Some("Fitness and Health")),
            ("instagram", &["This kurta is under Rs. 499 on Myntra, link in bio"], Some("Buy Later")),
            ("x", &["me after one meeting 😂"], Some("Funny")),
            ("web", &["Top 10 Pocket Alternatives in 2026"], Some("Read Later")),
            ("x", &["gm"], None),
        ];
        for (platform, words, want) in cases {
            assert_eq!(suggest_folder(platform, words), *want, "{words:?}");
        }
    }

    #[test]
    fn single_words_match_whole_words_only() {
        // "ai" must not match inside "said", nor "mistake" count as anything.
        assert_eq!(suggest_folder("x", &["he said it was a mistake"]), None);
        assert_eq!(suggest_folder("x", &["new AI tools"]), Some("Learn"));
    }

    #[test]
    fn every_rule_folder_is_a_listed_folder() {
        for (f, _) in RULES {
            assert!(FOLDERS.contains(f), "{f}");
        }
    }
}
