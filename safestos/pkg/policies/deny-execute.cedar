// deny-execute.cedar — refuse the build/load outright.
//
// The request the gate asks first is `Execute` on Sandbox::"host" for
// principal Cell::"builder". With this policy `avm-pkg build --policy
// deny-execute.cedar` exits 1 with
//   avm-pkg: EPERM: capability denied for action "Execute"
// and no tool process exists. Swap it for allow.cedar and the same
// command builds, which is what makes it a control rather than a guess.
forbid(principal, action, resource);
