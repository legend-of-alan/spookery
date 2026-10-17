# Expressions

A rule number is an expression. The pack loads it once. Play evaluates that tree. It does not parse text again.

These fields take an expression:

- `roll.target` and `roll.partial_at`
- a condition's `meter.value` and `clock.value`
- a meter effect's `add` and `set`
- a resist roll's `target`
- a cost's meter amounts and item counts

An integer is still an integer. `{ meter: { who, key }, plus: N }` is still that meter plus `N`.

## Forms

| Form | Meaning |
|---|---|
| `12`, `-4` | An integer. |
| `2d6` | Roll two six-sided dice and add the faces. |
| `2d20kh1`, `2d20kl1` | Roll two twenty-sided dice and keep the higher or lower face. |
| `self.might`, `target.hp`, `subject.grit` | That actor's meter. |
| `here.light` | The actor's place, then that id as an actor. |
| `world.hour` | Refused at play. The world has no meters. |
| `place:loft.light` | The named place, then that id as an actor. |
| `mira.grit` | That cast member's meter. |
| `a + b`, `a - b`, `a * b`, `a / b` | Arithmetic. `/` rounds toward negative infinity. |
| `min(a, b)`, `max(a, b)` | The lower or higher value. |
| `(a + b) * c` | Parentheses. |

`0d6`, `2d0`, a literal divisor of `0`, and a token the grammar does not accept fail `spook check` on the file and line of the expression. An unknown meter, or a `who` that is not `self`, `target`, `subject`, `here`, `world`, `place:<id>`, or a cast id, fails the same way.

Overflow is refused when the effect is applied. That effect changes nothing. Dividing by zero at play, when the divisor is not a literal `0`, is refused the same way.

Dice use the round's roll seed. Each die group in one expression continues that seed. The journal line records the faces, and the GM commit shows them, in the form `2d6 → 4, 5 = 9 + 3`. The player does not see the faces.
