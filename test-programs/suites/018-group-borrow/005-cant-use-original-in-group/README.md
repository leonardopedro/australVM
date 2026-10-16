Using an original of a group borrow inside the borrow body must be rejected by
the linearity checker, exactly like using the original of a single borrow: the
check runs on the desugared nested borrows.
