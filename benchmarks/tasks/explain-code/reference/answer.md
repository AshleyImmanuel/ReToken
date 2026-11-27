`Money.allocate(ratios)` works in integer minor units (cents for USD). It
converts the amount to minor units, remembers the sign and works on the
absolute value. Each part first gets the floor of its exact share,
`units * ratio // sum(ratios)`. Flooring leaves a remainder of fewer minor
units than there are parts; those leftover cents are handed out one each to
the parts with a non-zero ratio, in the order the ratios were given, so the
first parts absorb the rounding (100.00 USD over [1, 1, 1] gives 33.34, 33.33,
33.33). Parts with a zero ratio get nothing. Finally the sign is reapplied, so
a negative amount splits into negative parts (-1.00 gives -0.34, -0.33,
-0.33). The parts always sum to the original amount exactly. Empty ratios,
negative or non-integer ratios, or all-zero ratios raise `ValueError`.
