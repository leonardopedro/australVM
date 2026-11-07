#!/usr/bin/env perl
# closehyps.pl — automatic closed-form (deduction-theorem) companions for MM1.
#
# A theorem stated with essential hypotheses
#
#     pub theorem T (params) (h1: $ F1 $) ... (hn: $ Fn $) : $ C $ = 'proof';
#
# is not usable where a *closed* implication is needed: MM0 has no deduction
# rule, so a caller who wants `F1 -> ... -> C` cannot get it by assuming F1.
# This tool derives that companion mechanically
#
#     pub theorem T_c (params) : $ (wi F1 (wi F2 ( ... C ))) $ = 'proof';
#
# by the classical deduction theorem over the proof tree, using only set.mm's
# propositional axioms (ax_1, ax_2, ax_mp) and, for `Fk -> Fk`, set.mm's `id`.
# No step assumes a hypothesis: every node of the emitted proof is a closed
# instance.
#
# Per hypothesis Fk, closed from the last hypothesis backwards:
#   leaf hk            -> id                     (Fk -> Fk)
#   leaf hi (i<k)      -> (ax_mp hi ax_1)        (Fk -> Fi)
#   closed rule R      -> (ax_mp R ax_1)         (Fk -> concl R)
#   node (ax_mp a b)   -> (ax_mp Da (ax_mp Db ax_2))
#   node (R a1..am)    -> peel each arg off R's closed companion R_c:
#                         (ax_mp Dai (ax_mp acc ax_2))
#
# A cited rule R with hypotheses needs a closed companion R_c.  Local theorems
# (defined in the same .mm1) get one recursively.  An *external* rule with
# hypotheses (a set.mm theorem stated with $e hypotheses) does not: its closure
# is new mathematics, not a syntactic rewrite, so the tool reports it as part of
# the closure frontier rather than inventing one.  That frontier is the honest
# boundary of this transform, and the input to the next stage (feeding set.mm's
# own proofs, which are stored in compressed form).
#
# Usage:
#   mm0/tools/closehyps.pl FILE.mm1 [--spec mm0/metamath/set.mm0] [--only NAME]
#   mm0/tools/closehyps.pl FILE.mm1 --out closed_stmts.mm0
#
# Exit: 0 every requested theorem closed, 3 frontier nonempty (those theorems
# emit nothing), 1 parse error.

use strict;
use warnings;
use FindBin;
use lib $FindBin::Bin;
use SetMM ();

my $sdb;   # SetMM database, parsed on first use (see mma_db)

my ($file, $spec, $only, $out, $why, $mma, $max_rss) = (undef, "mm0/metamath/set.mm0", undef, undef, 0, undef, 0);
while (my $a = shift @ARGV) {
  if    ($a eq '--spec') { $spec = shift @ARGV }
  elsif ($a eq '--only') { $only = shift @ARGV }
  elsif ($a eq '--out')  { $out  = shift @ARGV }
  elsif ($a eq '--why')  { $why  = 1 }
  elsif ($a eq '--mma')  { $mma  = shift @ARGV }
  elsif ($a eq '--max-rss') { $max_rss = shift @ARGV }   # MB, default 75% of RAM

  elsif ($a =~ /^-/)     { die "unknown flag $a\n" }
  else                   { $file = $a }
}
die "usage: closehyps.pl FILE.mm1 [--spec set.mm0] [--only NAME] [--out FILE]\n"
  unless defined $file;
die "no such file: $file\n" unless -e $file;

sub trim { my $s = shift; $s =~ s/\A\s+//; $s =~ s/\s+\z//; return $s }

# ---------------------------------------------------------------- memory guard
# Parsing set.mm costs ~7 GB in Perl (measured), and every derived companion
# adds to that.  Letting the kernel kill the process loses the whole report, so
# the cascade stops at a budget instead and names the point it reached: that is
# a measured frontier, not a silent truncation.
my $rss_limit_mb;
sub mem_total_mb {
  open my $f, '<', '/proc/meminfo' or return 0;
  while (<$f>) { return int($1 / 1024) if /^MemTotal:\s+(\d+) kB/ }
  return 0;
}
sub rss_mb {
  open my $f, '<', '/proc/self/status' or return 0;
  while (<$f>) { return int($1 / 1024) if /^VmRSS:\s+(\d+) kB/ }
  return 0;
}
sub over_budget {
  $rss_limit_mb ||= $max_rss || int(mem_total_mb() * 0.75);
  my $mb = rss_mb();
  return ($mb > $rss_limit_mb) ? $mb : 0;
}

# ---------------------------------------------------------------- statement db
# label -> number of hypotheses
my %db;      # label -> { nh, forms => [...], concl => ... }
my ($sql, %ctor);
$sql = do {
  local $/;
  open my $sf, '<', $spec or die "cannot read spec $spec: $!";
  <$sf>;
};
$ctor{$1} = 1 while $sql =~ /^(?:term|sort)\s+([^\s{(:;]+)/mg;   # constructors
# statements are `theorem label (params): $ h1 $ > ... > $ concl $;`, wrapped
while ($sql =~ /^(?:theorem|axiom)\s+([^\s{(:]+)([^;]*);/mg) {
  my $label = $1;
  my $body  = $2;                        # capture before any other match
  my @forms = $body =~ /\$([^\x24]*)\$/g;   # `[^$]` would interpolate `$]`
  my $head  = $body;
  $head = $1 if $head =~ /\A(.*?):/s;
  my @pp;
  while ($head =~ /[({]\s*([^(){}:]*?)\s*:\s*([^(){}]*?)\s*[)}]/g) {
    my ($names, $type) = ($1, $2);
    push @pp, [ $_, $type ] for split ' ', $names;
  }
  next unless @forms;
  my $concl = pop @forms;
  s/^\s+|\s+$//g for ($concl, @forms);
  $db{$label} = { nh => scalar(@forms), forms => [@forms], concl => $concl, params => [@pp] };
}
die "spec $spec yielded no statements\n" unless %db;

# ---------------------------------------------------------------- mm1 parsing
my $text = do {
  local $/;
  open my $fh, '<', $file or die "cannot read $file: $!";
  <$fh>;
};

my (%local, @order);
my @chunks = split /(?=^pub\s+theorem\s)/m, $text;
for my $c (@chunks) {
  next unless $c =~ /\Apub\s+theorem\s+([^\s{:(]+)/;
  my $name = $1;

  my $hash = index($c, '=');
  next if $hash < 0;
  my $header = substr($c, 0, $hash);
  my $rest   = substr($c, $hash + 1);
  my $q1 = index($rest, "'");              # a proof is `'<s-expression>`
  next if $q1 < 0;
  my $q2 = index($rest, ';', $q1);        # the statement terminator
  next if $q2 < 0;
  my $proof = substr($rest, $q1 + 1, $q2 - $q1 - 1);

  # header is `pub theorem NAME <params> (h1: $ F1 $) ... : $ C $`; every
  # `$ ... $` group is preceded by the `name:` that introduces it
  my @parts = split /\$/, $header;         # odd indices are formulas
  my (@forms, @hn);
  for (my $j = 1; $j < @parts; $j += 2) {
    push @forms, trim($parts[$j]);
    my $nm;
    $nm = $1 if $parts[$j - 1] =~ /([A-Za-z_]\w*)\s*:\s*\z/;
    push @hn, $nm;
  }
  next unless @forms;
  my $concl = pop @forms;
  pop @hn;                                  # the conclusion has no name

  my $params = $parts[0];
  $params =~ s/\Apub\s+theorem\s+\S+\s*//;
  $params =~ s/\(\s*\w+\s*:\s*\z//;       # the `(h1:` of the first hypothesis
  $params = trim($params);

  my $hi = 0;
  for my $h (@hn) { $h = defined $h ? $h : "h" . (++$hi) }

  die "hyp count mismatch in $name\n" unless @hn == @forms;

  $local{$name} = {
    name  => $name,
    params => $params,
    hyps  => [@hn],
    forms => [@forms],
    concl => $concl,
    tree  => parse_proof($proof, $name),
    nh    => scalar(@forms),
  };
  push @order, $name;
}

sub parse_proof {
  my ($s, $name) = @_;
  my @tok = $s =~ /\(|\)|[^\s()]+/g;
  my $i = 0;
  my $node;
  $node = sub {
    my $t = $tok[$i++];
    die "unexpected end of proof in $name\n" unless defined $t;
    return $t if $t ne '(';
    my $head = $tok[$i++];
    die "empty application in proof of $name\n" unless defined $head;
    my @args;
    while (defined $tok[$i] && $tok[$i] ne ')') { push @args, $node->() }
    die "unbalanced proof in $name\n" unless defined $tok[$i];
    $i++;                                   # consume ')'
    return [$head, @args];
  };
  my $r = $node->();
  die "trailing tokens in proof of $name\n" if $i < @tok;
  return $r;
}

sub unparse {
  my ($t) = @_;
  # Decoder nodes carry their instantiated conclusion beside the arguments; it
  # is evidence for the dependent-hypothesis case, not proof syntax.
  if (ref $t eq 'HASH') {
    return "(" . join(" ", $t->{name}, map { unparse($_) } @{ $t->{args} }) . ")";
  }
  return leaf_name($t) unless ref $t;
  return "(" . join(" ", $t->[0], map { unparse($_) } @{$t}[1 .. $#{$t}]) . ")";
}

# A `$f` label names a variable; proofs of terms built from set.mm carry those
# labels as leaves, but MM1 wants the variable itself.
sub leaf_name {
  my ($l) = @_;
  return $l unless $sdb && $sdb->{st}{$l} && $sdb->{st}{$l}{type} eq 'f';
  return (split ' ', $sdb->{st}{$l}{concl})[-1];
}

sub hyps_of {
  my ($label) = @_;
  return 0 if $ctor{$label};   # a syntax constructor has no hypotheses
  return $local{$label}{nh} if $local{$label};
  return $db{$label}{nh}    if defined $db{$label};
  die "unknown statement '$label'\n";
}

# ------------------------------------------------- harvesting closed forms
# A rule R with hypotheses H1..Hn and conclusion C has the closed form
# `H1 -> ... -> Hn -> C`.  set.mm often states that shape directly, under a
# different label (ax_1 is the closed form of a1i, pm3.2 of pm3_2i, ...).
# Shape matching is up to a bijective renaming of the variables, so both sides
# are canonicalised by replacing every non-constructor token with the index of
# its first occurrence; the shapes then compare as plain strings.

sub sexpr {
  my ($s) = @_;
  my @tok = $s =~ /\(|\)|[^\s()]+/g;
  my $i = 0;
  my $node;
  $node = sub {
    my $t = $tok[$i++];
    return $t unless defined $t && $t eq '(';
    my @args;
    while (defined $tok[$i] && $tok[$i] ne ')') { push @args, $node->() }
    $i++ if defined $tok[$i];
    return @args == 1 ? $args[0] : \@args;   # a one-element group is just grouping
  };
  my $r = $node->();
  return $r;
}

# canonical shape string: variables -> #k in order of first occurrence
sub shape {
  my ($t) = @_;
  my %seen;
  my $go;
  $go = sub {
    my $n = shift;
    if (ref $n) { return "(" . join(" ", map { $go->($_) } @$n) . ")" }
    return $n if $ctor{$n};
    $seen{$n} = scalar(keys %seen) + 1 unless exists $seen{$n};
    return "#" . $seen{$n};
  };
  return $go->($t);
}

# MM0 formulas at top level are applications written without outer parens
sub parse_form { my $s = shift; return sexpr("($s)") }

my %by_shape;    # shape of a closed statement's conclusion -> label
for my $l (sort keys %db) {       # sorted, so the choice is reproducible
  next unless $db{$l}{nh} == 0;
  my $s = shape(parse_form($db{$l}{concl}));
  next if exists $by_shape{$s} and $by_shape{$s} =~ /^ax_/;
  next if exists $by_shape{$s} and $l !~ /^ax_/;
  $by_shape{$s} = $l;
}
printf STDERR "harvest index: %d closed shapes over %d statements\n",
  scalar(keys %by_shape), scalar(keys %db);

# the closed form of a rule: H1 -> ( ... -> C )
sub closure_formula {
  my ($label) = @_;
  my $d = $db{$label} or return undef;
  my $f = $d->{concl};
  $f = "($f)" unless $f =~ /^\(/;
  for (my $i = $#{$d->{forms}}; $i >= 0; $i--) {
    my $h = $d->{forms}[$i];
    $h = "($h)" unless $h =~ /^\(/;
    $f = "(wi $h $f)";
  }
  return $f;
}

# a proof of the closed form of an external rule, or undef.  Either set.mm
# states that shape directly, or it states both halves of an `ax_mp` step:
# one closed lemma for the antecedent and another for `antecedent -> goal`.
sub harvest {
  my ($label) = @_;
  my $f = closure_formula($label) or return undef;
  my $s = shape(parse_form($f));
  printf STDERR "  %s closure shape: %s\n", $label, $s if $why;
  return $by_shape{$s} if exists $by_shape{$s};

  my @closed = sort { ($b =~ /^ax_/) <=> ($a =~ /^ax_/) or $a cmp $b }
               grep { $db{$_}{nh} == 0 } keys %db;
  for my $a (@closed) {
    my $ac = $db{$a}{concl};
    $ac = "($ac)" unless $ac =~ /^\(/;
    my $want = shape(parse_form("(wi $ac $f)"));
    my $b = $by_shape{$want} or next;
    return ['ax_mp', $a, $b];
  }
  return undef;
}

# ------------------------------------------------- closing external rules
# A rule set.mm states with hypotheses has no hypothesis-free counterpart, so
# `harvest` cannot find one.  Its closed form is derived here from set.mm's own
# proof of the rule, with the same deduction transform applied to the proof tree
# the decoder reconstructs.  Term arguments are not antecedents: only hypothesis
# arguments become implications, which keeps `class`/`setvar` parameters where
# MM0 expects them.

my (%extcache, %extbusy, @ext_emit, %fvar, $ext_n, $outh);

sub mma_db {
  return $sdb if $sdb;
  die "external closure needs --mma PATH\n" unless defined $mma;
  $sdb = SetMM::parse($mma);
  return $sdb;
}

sub var_map {                  # spec variable names -> set.mm variable names
  my ($a, $b, $map) = @_;
  return unless defined $a && defined $b;
  if (!ref $a && !ref $b) {
    return if $a eq $b;
    $map->{$a} = $b unless exists $map->{$a};
    return;
  }
  return unless ref $a && ref $b && @$a == @$b;
  var_map($a->[$_], $b->[$_], $map) for 0 .. $#$a;
}

sub ext_hyps {                 # `$e` hypothesis labels, in declaration order
  my ($l) = @_;
  mma_db();
  return [ map { $_->[0] } grep { $_->[1] eq 'e' } @{ $sdb->{st}{$l}{hyps} || [] } ];
}

# set.mm labels that are not MM0 labels: a leading digit is escaped and `-`
# becomes `_` (as does `.`), so set.mm's `ax-mp`, `pm5.74ri` and
# `3bitr4g` are the spec's `ax_mp`, `pm5_74ri`,
# `_3bitr4g`.
my %sname;
sub sname {
  my ($l) = @_;
  return $sname{$l} if exists $sname{$l};
  my $c = $l;
  $c = '_' . $c if $c =~ /^[0-9]/;
  $c =~ s/[-.]/_/g;   # `ax-mp` and `pm5.74ri`
  return $sname{$l} = $c;
}

# Is this set.mm label an axiom (`$a`) rather than a theorem (`$p`)?
#
# The distinction decides how a citation is built: a theorem's hypotheses are
# premises to peel, while an axiom has no proof to derive a companion from, and
# set.mm0 gives the quantifier axioms a Π-bound hypothesis
# (`axiom ax_gen {x3: setvar} (ph: wff x3): $ ph $ > $ wal x3 ph $;`) that the
# proof stream never pushes. Counting `$e` does not separate the two cases—
# set.mm's `ax-gen` sits in a scope carrying one `$e`—so the statement kind is
# what is asked, not the count.
sub is_axiom {
  my ($mm) = @_;
  mma_db();
  my $d = $sdb->{st}{$mm};
  return $d && $d->{type} eq 'a' ? 1 : 0;
}

# Is this label an axiom stated with a Π-bound parameter rather than premises?
#
# set.mm0 writes the quantifier axioms that way:
#   axiom ax_gen {x3: setvar} (ph: wff x3): $ ph $ > $ wal x3 ph $;
# Such an axiom is neither closeable (it has no proof) nor peelable (the
# "hypothesis" is the wff itself), so it is always cited directly, with its
# argument taken from the proof term (in `D`) or from the step's conclusion
# (in `XD`). `ax-mp` is the one axiom with real premises and is handled above.
sub is_param_axiom {
  my ($l) = @_;
  my $spec = sname(mm_label($l));
  return 0 if $ctor{$spec};
  return 0 unless hyps_of($spec);
  return is_axiom(mm_label($l));
}

# The set.mm label for a label that may already be a spec label.
#
# `companion` is called with the labels the local `.mm1` writes, i.e. set.mm0's
# spelling (`ax_gen`, `pm5_74ri`), while decoding needs set.mm's (`ax-gen`,
# `pm5.74ri`). `sname` goes one way only, so the inverse is built once, lazily.
my %back;
sub mm_label {
  my ($l) = @_;
  mma_db();
  return $l if $sdb->{st}{$l};
  unless (%back) {
    for my $mm (keys %{ $sdb->{st} }) {
      my $s = sname($mm);
      $back{$s} = $mm if $s ne $mm && !exists $back{$s};
    }
  }
  return $back{$l} // $l;
}

# The dependent arguments of such an axiom, recovered from the step's own
# conclusion: unify the axiom's conclusion pattern against it, then instantiate
# each hypothesis formula under the bindings.
sub _dep_args {
  my ($spec, $res) = @_;
  my $trace = sub { warn sprintf("dep[%s]: %s\n", $spec, $_[0]) if $ENV{MM_TRACE_XD} };
  unless (ref $res) {
    $trace->('the step carries no conclusion');
    return undef;
  }
  my $pat = sexpr("($db{$spec}{concl})");
  unless ($pat) {
    $trace->('the spec statement has no parseable conclusion');
    return undef;
  }
  my %bind;
  unless (_dep_unify($pat, $res, \%bind)) {
    $trace->(sprintf('pattern %s does not unify with the step %s', _dep_emit($pat), _dep_emit($res)));
    return undef;
  }
  my @out;
  for my $form (@{ $db{$spec}{forms} }) {
    my $v = _dep_inst(sexpr($form), \%bind);
    unless (defined $v) {
      $trace->(sprintf('hypothesis `%s` is not determined by the conclusion', $form));
      return undef;
    }
    push @out, _dep_emit($v);
  }
  $trace->(sprintf('recovered [%s]', join(', ', @out)));
  return \@out;
}

sub _dep_unify {
  my ($pat, $res, $bind) = @_;
  return 0 unless defined $res;
  if (!ref $pat) {
    # A nullary constructor must match itself; anything else is a parameter and
    # takes the step's subterm as its value.
    if ($ctor{$pat}) { return (!ref $res && sname($res) eq $pat) ? 1 : 0 }
    $bind->{$pat} = $res;
    return 1;
  }
  return 0 unless ref $res;
  my ($ph, @pa) = @$pat;
  my ($rh, @ra) = ref $res eq 'HASH'
    ? ($res->{name}, @{ $res->{args} })
    : @$res;
  return 0 unless sname($rh) eq $ph;
  return 0 unless @pa == @ra;
  for my $i (0 .. $#pa) { return 0 unless _dep_unify($pa[$i], $ra[$i], $bind) }
  return 1;
}

sub _dep_inst {
  my ($t, $bind) = @_;
  return $bind->{$t} if !ref $t && exists $bind->{$t};
  return $t unless ref $t;
  my @a = map { _dep_inst($_, $bind) } @$t;
  return (grep { !defined } @a) ? undef : \@a;
}

sub _dep_emit {
  my ($t) = @_;
  return leaf_name($t) unless ref $t;
  return "(" . join(" ", map { _dep_emit($_) } @$t) . ")";
}

# D, but a rule's term arguments are copied, not peeled: they are not hypotheses
sub XD {
  my ($tree, $t, $k) = @_;
  if (!ref $tree) {
    my $i;
    for my $j (0 .. $#{ $t->{hyps} }) {
      $i = $j + 1 if defined $t->{hyps}[$j] && $t->{hyps}[$j] eq $tree;
    }
    return $i == $k ? 'id' : ['ax_mp', $tree, 'ax_1'] if $i;
    return ['ax_mp', $tree, 'ax_1'];
  }
  my ($head, @args, $res);
  if (ref $tree eq 'HASH') { $head = $tree->{name}; @args = @{ $tree->{args} }; $res = $tree->{res} }
  else                     { ($head, @args) = @$tree }
  if ($head eq 'ax_mp' || $head eq 'ax-mp') {   # set.mm spells it `ax-mp`
    return ['ax_mp', XD($args[0], $t, $k), ['ax_mp', XD($args[1], $t, $k), 'ax_2']];
  }
  my $spec = sname($head);            # set.mm label -> set.mm0 label
  my $nh = hyps_of($spec);
  warn sprintf("XD head=%s spec=%s nh=%d axiom=%d\n", $head, $spec, $nh, is_axiom($head))
    if $ENV{MM_TRACE_XD};
  # A syntax constructor takes its term arguments explicitly and is an atom of
  # the closure; a hypothesis-free theorem is cited by name (MM0 unifies its
  # parameters), exactly as MM0 itself expects.
  return ['ax_mp', $ctor{$spec} ? [$spec, @args] : $spec, 'ax_1'] unless $nh;
  # An axiom is cited, never closed (`ax-mp` is handled far above). set.mm0's
  # quantifier axioms carry a Π-bound hypothesis instead of a premise, so the
  # argument is recovered from the step's own conclusion — which the decoder
  # records — and the axiom is applied directly, as an atom of the closure.
  if (is_param_axiom($head)) {
    # A node synthesized by an earlier pass already carries its argument and
    # has no conclusion of its own; copy what is there, as for a constructor.
    return ['ax_mp', [$spec, @args], 'ax_1'] unless $res;
    my $dep = _dep_args($spec, $res)
      or die "cannot recover the dependent arguments of $spec\n";
    return ['ax_mp', [$spec, @$dep], 'ax_1'];
  }
  my @hyp = @args ? @args[ -$nh .. -1 ] : ();
  my $acc = ['ax_mp', external_closure($head)->{name}, 'ax_1'];
  for my $a (@hyp) { $acc = ['ax_mp', XD($a, $t, $k), ['ax_mp', $acc, 'ax_2']] }
  return $acc;
}

sub external_closure {
  my ($label) = @_;
  return $extcache{$label} if $extcache{$label};
  die "recursive closure of external rule $label\n" if $extbusy{$label};
  $extbusy{$label} = 1;
  my $r = eval { _external_closure($label) };
  my $err = $@;
  delete $extbusy{$label};                   # also when it failed
  die $err if $err;
  return $extcache{$label} = $r;
}

sub _external_closure {
  my ($mm)  = @_;
  $mm = mm_label($mm);                # accept either spelling
  my $label = sname($mm);             # the set.mm0 label
  if (my $mb = over_budget()) {
    die sprintf("memory budget reached (%.1f GB > %.1f GB) after %d closures\n",
                $mb / 1024, $rss_limit_mb / 1024, $ext_n);
  }
  my $nh = hyps_of($label);
  die "external rule $label has no hypotheses\n" unless $nh;
  mma_db();
  my $d = SetMM::derive($sdb, $mm)
    or die "cannot decode set.mm proof of $mm\n";
  my $t = { name => $label, nh => $nh, hyps => ext_hyps($mm),
            forms => [ @{ $db{$label}{forms} } ], concl => $db{$label}{concl} };
  my $pt = $d->{proof_tree} or die "decoder gave no proof tree for $label\n";
  $pt = XD($pt, $t, $_) for reverse 1 .. $nh;

  my %map;
  var_map(sexpr("($db{$label}{concl})"),
          SetMM::_form_tree($sdb, $sdb->{st}{$mm}{concl}), \%map);
  my $form = closure_formula($label);
  for my $v (keys %map) {
    my $m = $map{$v};
    $form =~ s/(?<![A-Za-z0-9_])\Q$v\E(?![A-Za-z0-9_])/$m/g;
  }
  my @pp;
  for my $prm (@{ $db{$label}{params} || [] }) {
    my ($nm, $ty) = @$prm;
    $nm = $map{$nm} if exists $map{$nm};
    $ty =~ s/(?<![A-Za-z0-9_])([A-Za-z0-9_]+)/exists $map{$1} ? $map{$1} : $1/ge;
    push @pp, "($nm: $ty)";
  }
  my $name = "${label}_c";
  # The statement above and the proof below are both spelled the way set.mm
  # spells the variables (%map renames spec -> set.mm), which is the same
  # convention `unparse` uses for the decoded leaves.
  my $stmt = "pub theorem $name " . (@pp ? join(" ", @pp) . " " : "")
           . ": \$ $form \$ = '" . unparse($pt) . ";";
  $stmt =~ s/\s+/ /g;
  # Each statement is written as it is produced: a full cascade is thousands of
  # theorems, and holding them costs memory the cascade itself needs.
  $ext_n++;
  if ($outh) { print $outh "$stmt\n" } else { push @ext_emit, $stmt }
  printf STDERR "  closed external rule %-10s -> %-14s (%d hypothesis/es)\n",
    $label, $name, $nh;
  return { name => $name };
}

# ---------------------------------------------------------------- the transform

my (%closed, %frontier, %busy, %harvested, %said);

sub close_local;   # forward

sub D;             # forward

# label string usable as a proof of the closed form of $label
sub companion {
  my ($label) = @_;
  return $label if hyps_of($label) == 0;            # already closed
  if ($local{$label}) { return close_local($label)->{name} }
  # external rule stated with hypotheses: set.mm may state its closed form
  # directly under another label, in which case no new mathematics is needed
  if (my $h = harvest($label)) { $harvested{$label} = $h; return $h }
  # otherwise record it and keep walking, so the report lists the whole
  # frontier for the theorem rather than its first hit
  if (my $e = eval { external_closure($label) }) { return $e->{name} }
  # A frontier entry is a *failed* closure, and the reason is what makes it
  # actionable: report it, rather than only the label.
  if ($@) {
    my $why = $@ =~ s/\n.*//sr;
    printf "  (%s: %s)\n", $label, $why unless $said{$why}++;
  }
  $frontier{$label} = 1;

  return $label;
}

sub close_local {
  my ($name) = @_;
  return $closed{$name} if $closed{$name};
  die "recursive closure of $name\n" if $busy{$name}++;
  my $t = $local{$name};

  my $tree = $t->{tree};
  for (my $k = $t->{nh}; $k >= 1; $k--) { $tree = D($tree, $t, $k) }

  # a formula used as an argument must be parenthesised
  my $arg = sub { my $f = shift; return $f =~ /^\(/ ? $f : "($f)" };
  my $form = $arg->($t->{concl});
  for (my $i = $t->{nh} - 1; $i >= 0; $i--) {
    $form = "(wi " . $arg->($t->{forms}[$i]) . " $form)";
  }
  $closed{$name} = { name => "${name}_c", formula => $form, proof => $tree };
  delete $busy{$name};
  return $closed{$name};
}

# D(tree, theorem, k): derive `Fk -> <formula of tree>` without assuming Fk
sub D {
  my ($tree, $t, $k) = @_;

  if (!ref $tree) {
    my $i;
    for my $j (0 .. $t->{nh} - 1) { $i = $j + 1 if defined $t->{hyps}[$j] && $t->{hyps}[$j] eq $tree }
    if ($i) { return $i == $k ? 'id' : ['ax_mp', $tree, 'ax_1'] }
    # a bare hypothesis-free citation, e.g. `ax_1` or `id`
    die "unknown leaf '$tree' in $t->{name}\n" if hyps_of($tree) != 0;
    return ['ax_mp', $tree, 'ax_1'];
  }

  my ($head, @args) = @$tree;
  if ($head eq 'ax_mp' || $head eq 'ax-mp') {
    return ['ax_mp', D($args[0], $t, $k), ['ax_mp', D($args[1], $t, $k), 'ax_2']];
  }

  # A Π-parameter axiom is cited with its argument as written: the proof term
  # already supplies the bound wff, and there is no premise to peel.
  if (is_param_axiom($head)) {
    return ['ax_mp', [sname(mm_label($head)), @args], 'ax_1'];
  }

  my $base = companion($head);
  my $acc = ['ax_mp', $base, 'ax_1'];               # Fk -> concl(rule)
  for my $a (@args) {
    $acc = ['ax_mp', D($a, $t, $k), ['ax_mp', $acc, 'ax_2']];
  }
  return $acc;
}

# ---------------------------------------------------------------- emit
my @targets = defined $only ? ($only) : grep { $local{$_}{nh} > 0 } @order;
my (@emit, $failed);
if (defined $out) {
  open $outh, '>', $out or die "cannot write $out: $!\n";
  select((select($outh), $| = 1)[0]);      # stream, so a stop still leaves output
}

for my $name (@targets) {
  my $t = $local{$name} or die "no local theorem '$name'\n";
  next if $t->{nh} == 0;
  %closed = ();
  %frontier = ();
  eval { close_local($name) };
  if (%frontier) {
    printf STDERR "%-44s needs closures: %s\n", $name, join(", ", sort keys %frontier);
    $failed++;
    next;
  }
  die $@ if $@;
  my $c = $closed{$name};
  my $stmt = "pub theorem $c->{name} $t->{params} : \$ $c->{formula} \$ = '"
           . unparse($c->{proof}) . ";";
  $stmt =~ s/\s+/ /g;
  push @emit, $stmt;
  delete $closed{$name};
}

if ($outh) {
  print $outh "$_\n" for @emit;
  close $outh;
  printf STDERR "wrote %d statement(s) to %s (%d external, %d local)\n",
    $ext_n + scalar(@emit), $out, $ext_n, scalar(@emit);
} else {
  print "$_\n" for (@ext_emit, @emit);
}
printf STDERR "closed=%d frontier=%d\n", scalar(@emit), $failed || 0;
if (open my $ps, '<', '/proc/self/status') {
  while (<$ps>) { printf STDERR "peak RSS %d MB\n", int($1 / 1024) if /^VmHWM:\s+(\d+) kB/ }
}
exit($failed ? 3 : 0);
