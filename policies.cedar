permit(
    principal == Module::"test_cps::main",
    action == Action::"Call",
    resource == Module::"SafeFunc"
);

forbid(
    principal == Module::"test_cps::main",
    action == Action::"Call",
    resource == Module::"ForbiddenFunc"
);
