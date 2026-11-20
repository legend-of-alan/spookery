# Expressions

A rule number is an expression. The pack loads it once. Play evaluates that tree. It does not parse text again.

Every number in a rule line is an expression:

- a check's `total`, `success_at` and `partial_at`, on a rule or nested under a dash
- a meter effect's `add` and `set`, and an item effect's `add` and `max`
- a modifier effect's `add` and `ticks`
- a condition's numbers: `has_item.count`, `meter.value`, `players.value`, and `cycle.from` and `cycle.to`
- a cost's `ticks`, meter amounts and item counts, on a rule or a path

A rule's `let:` numbers are expressions too, and the rule's other numbers read them by id.

A number in a condition or a cost cannot roll dice; dice there fail `spook check` on that line. A `let:` id counts as dice, because the gate reads a condition and a cost before `let:` rolls, so an id read there fails the same way. An integer is still an integer.

## Forms

| Form | Meaning |
|---|---|
| `12`, `-4` | An integer. |
| `2d6` | Roll two six-sided dice and add the faces. |
| `2d20kh1`, `2d20kl1` | Roll two twenty-sided dice and keep the higher or lower face. |
| `self.might`, `target.hp` | That actor's meter. |
| `here.light` | The actor's place's meter. |
| `world.dawn` | The world's meter, declared in `world/meters.yaml`. |
| `place:loft.light` | The named place's meter, as for `here`. |
| `story:ending.dread` | The named story's meter. |
| `mira.vigor` | That cast member's meter. |
| `attack` | The value the rule's `let:` stored under that id. |
| `high(dmg)`, `low(dmg)` | The higher or lower face of a `let:` id whose number is one dice group with no keep, such as `dmg: 2d8`. |
| `a + b`, `a - b`, `a * b`, `a / b` | Arithmetic. `/` rounds toward negative infinity. |
| `min(a, b)`, `max(a, b)` | The lower or higher value. |
| `(a + b) * c` | Parentheses. |

A bare word is a `let:` id, so it never names a meter or an owner. A `let:` number reads only the ids written above it. An id the rule's `let:` does not hold, an id a `let:` number reads at or above the line that holds it, and `high` or `low` of a number that is not one dice group with no keep fail `spook check` at the rule's line. A `let:` id that is not such a bare word, or that is an owner word or `min`, `max`, `high` or `low`, fails at its own line.

Because `/` rounds toward negative infinity, halving a loss rounds the loss up: `-(1d8 + self.might) / 2` turns 7 into −4. `-((1d8 + self.might) / 2)` turns it into −3.

`0d6`, `2d0`, more than 1000 dice in one group, a literal divisor of `0`, and a token the grammar does not accept fail `spook check` on the file and line of the expression. An unknown meter, a meter that its fixed entity (`world`, `place:<id>`, `story:<id>`, a cast id, or `self` on a cast member's own habit) does not carry, or an owner that is not `self`, `target`, `here`, `world`, `place:<id>`, `story:<id>`, or a cast id, fails the same way.

Overflow is refused when the effect is applied. That effect changes nothing. Dividing by zero at play, when the divisor is not a literal `0`, is refused the same way.

Dice derive from the world's seed and the round. A save with no `seed:` gets a fresh one the first time the world opens for play, before its first round, so two copies of a shipped pack roll differently. Writing `seed:` into `state/save.yaml` pins the dice. Each die group in one expression continues where the last one stopped. The journal line records the faces, and the GM commit shows them, in the form `2d6 → 4, 5 = 9 + 3`. A `let:` id that rolls writes its line with its id in front, `attack: 1d20 → 14 = 14 + 3`; reading it writes nothing. The player does not see the faces.
