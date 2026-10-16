A group write borrow of a `let` (immutable) original must be rejected, exactly
like a single write borrow: the desugaring produces single borrows, so the
mutability check fires on the desugared form.
