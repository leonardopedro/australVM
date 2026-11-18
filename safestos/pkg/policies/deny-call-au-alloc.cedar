// deny-call-au-alloc.cedar — the cell may run, but not make one call.
//
// `avm-pkg load` asks `Execute` and then one `Call:<symbol>` request per
// import the cell declares. The bootstrap cell built from
// stdlib/pkg/{Derivation,Bootstrap}.aum imports `au_alloc`, so with this
// policy the load is refused at that call and *not* at Execute:
//   avm-pkg: EPERM: capability denied for action "Call:au_alloc"
// Every other call stays permitted, which is the point of asking per
// symbol instead of once per cell.
permit(principal, action, resource);
forbid(principal, action == Action::"Call:au_alloc", resource);
