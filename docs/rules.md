# Standard Kniffel rules used by the engine

Source: the standard section of the Schmidt Spiele rulebook (Kniffel, item 49030, German; pages 2–3 of its PDF, which is not part of this repository). Saved positions use `schmidt_pdf_2024_house_v2`: these rules with the house conventions below. Files saved as `schmidt_pdf_2024_standard_v1` (before five of a kind counted as a Full House) load as current positions, since their sheets are valid under both; models and caches trained under v1 are rejected.

| JSON index / TUI number | Category | Points |
| --- | --- | --- |
| 0–5 / 1–6 | Ones through Sixes | Sum of that face |
| 6 / 7 | Three of a kind | Sum of all five dice if at least three equal |
| 7 / 8 | Four of a kind | Sum of all five dice if at least four equal |
| 8 / 9 | Full House | 25 for a triple and a pair, including five of a kind (house rule) |
| 9 / 10 | Small Straight | 30 for four distinct consecutive faces |
| 10 / 11 | Large Straight | 40 for five distinct consecutive faces |
| 11 / 12 | Kniffel | 50 for five equal dice |
| 12 / 13 | Chance | Sum of all five dice |

The upper section earns 35 extra points once its subtotal reaches 63. A large straight also satisfies the small-straight condition. Duplicate dice do not invalidate an otherwise present small straight. Five equal dice are also a Full House (house rule 3 below).

Each turn starts with five dice and allows up to two rerolls. Any subset can be rerolled, and previously retained dice may be released at a later decision. Scoring early is allowed. Each turn consumes one open category, including a zero entry. The game ends when everyone's 13 categories are filled.

An eligible additional Kniffel earns 50 extra points plus the maximum in any open category, regardless of face: upper entries are five times the category face; Three/Four of a Kind and Chance are 30; Full House and straights use their fixed points. The extra bonus is awarded once on scoring the turn. Retaining the same five dice does not repeatedly award it.

Two conventions make underspecified wording in the PDF explicit, and a third is the family's house rule:

1. Additional-Kniffel eligibility requires **50 already entered in Kniffel**. Crossing out Kniffel disables extra bonuses. A previous five-of-a-kind entered elsewhere does not establish eligibility. The PDF does not spell out these history cases; this is the documented implementation convention.
2. Every entry is the player's choice. A category the roll does not fit receives zero; crossing out with zero is always allowed, also where the roll fits; and in the upper section fewer of the matching dice may count (five ones may be entered as 1–5 in Ones). Chance always fits a five-dice roll. A roll that earns an extra Kniffel earns its 50 points whatever is entered. Analysis only considers the full score where the roll fits, otherwise crossing out: the other entries never score more, so they are never the better move.
3. Five of a kind counts as a Full House: a triple plus a pair, for the usual 25 points, whether or not Kniffel is still open. The PDF's wording („drei gleiche und zwei gleiche, andere Zahlen“) would exclude it; the family plays it this way.

`null` in a saved sheet means open; `0` means filled for zero. `extra_kniffels` counts additional awards, not all five-of-a-kind throws and not points. Totals and the upper bonus are derived. Input validation checks category score possibilities, bonus consistency, phases, and ordinary turn counts. Without a complete move history it cannot prove that every entered sheet was reached in actual play.
