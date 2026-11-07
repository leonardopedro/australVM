# SetMM.pm — read a Metamath source file, including its compressed proofs.
#
# Only what proof reconstruction needs is kept: for every statement its ordered
# mandatory hypotheses (label + $f/$e), its conclusion, and for $p statements
# the raw proof section.  Formulas are kept as whitespace-normalised token
# strings in Metamath's own syntax.
#
# The compressed-proof format (Metamath book, Appendix B, and mmpars.c) is
#
#     label $p <conclusion> $= ( <label hints> ) <letters> $.
#
# The letters index a *label map*, and each decoded reference is either a
# hypothesis (push), a local label (share the step it names), or an assertion,
# which pops one stack entry per mandatory hypothesis and pushes its result.
# A reference is written as base-5 high digits U..Y (optional) followed by one
# base-20 low digit A..T, which terminates the reference:
#
#     value = 0
#     for c in token: value = (c in U..Y) ? value*5 + (c-'U'+1)
#                                   : value*20 + (c-'A')
#
# `Z` names the step just completed, so later references can reuse it instead
# of re-deriving it.

package SetMM;

use strict;
use warnings;

# ---------------------------------------------------------------- tokenising
# Comments are `$( ... $)` and may contain anything except `$)`.  Stripping
# them up front keeps the token loop free of state.
sub _strip_comments {
  my ($s) = @_;
  $s =~ s/\$\((?:[^\x24]|\x24(?!\)))*\$\)//g;
  return $s;
}

sub parse {
  my ($path) = @_;
  open my $fh, '<', $path or die "cannot read $path: $!\n";
  my $s = do { local $/; <$fh> };
  close $fh;
  $s = _strip_comments($s);

  my (%st, @order, %isvar);
  my @scopes = ([]);        # set.mm declares the global $f hypotheses at top level
  my ($pending, $mode, @buf) = (undef, 'idle');

  # each scope: [ [label, type, formula], ... ] in declaration order
  my $flush = sub {
    my ($tok) = @_;
    return unless defined $pending;
    my @hyps;
    for my $sc (@scopes) { push @hyps, @$sc }
    $st{$pending} ||= {};        # a $p already recorded its conclusion
    $st{$pending}{type} = $tok;
    # The `[label, type, formula]` scope entries are *shared*, not copied: they
    # live on through the statements that name them, and readers only ask for
    # `$_->[0]`/`$_->[1]`.  Copying them per statement duplicated 22.6M pairs
    # over set.mm, which is where most of the parse's several GB went.
    $st{$pending}{hyps} = [@hyps];
    $st{$pending}{concl} = '' unless defined $st{$pending}{concl};
    return;
  };

  while ($s =~ /\G\s*(\S+)/gc) {
    my $t = $1;

    # statements are terminated by `$.`; `$;` is the older spelling
    if ($mode eq 'skip_semi') { $mode = 'idle' if $t eq '$;' || $t eq '$. ' || $t eq '$.'; next }
    if ($mode eq 'a' || $mode eq 'e' || $mode eq 'f') {
      if ($t eq '$;' || $t eq '$.' || $t eq '$. ') {
        my $f = join(' ', @buf); @buf = ();
        if ($mode eq 'a') {
          $flush->('a');
          $st{$pending}{concl} = $f;
          push @order, $pending;
        } elsif ($mode eq 'e') {
          $flush->('e');
          $st{$pending}{concl} = $f;
          push @{ $scopes[-1] }, [$pending, 'e', $f];
        } elsif ($mode eq 'f') {
          $flush->('f');
          $st{$pending}{concl} = $f;
          push @{ $scopes[-1] }, [$pending, 'f', $f];
        }
        $mode = 'idle'; $pending = undef;
        next;
      }
      push @buf, $t;
      next;
    }
    if ($mode eq 'p_concl') {
      if ($t eq '$=') { $st{$pending}{concl} = join(' ', @buf); @buf = (); $mode = 'p_proof'; next }
      push @buf, $t;
      next;
    }
    if ($mode eq 'p_proof') {
      if ($t eq '$.') {
        $st{$pending}{proof} = join(' ', @buf); @buf = ();
        $flush->('p');
        $st{$pending}{concl} = $st{$pending}{concl};   # set in p_concl
        push @order, $pending;
        $mode = 'idle'; $pending = undef;
        next;
      }
      push @buf, $t;
      next;
    }

    # idle: `label $x ...` or a directive
    if ($t eq '${') { push @scopes, []; next }
    if ($t eq '$}') { pop @scopes if @scopes > 1; next }
    if ($t eq '$c' || $t eq '$v') {
      $mode = 'skip_semi';
      # $v declares the variables, which shape comparison must not treat as
      # constructors
      if ($t eq '$v') {
        pos($s) = pos($s);
        my $save = pos($s);
        while ($s =~ /\G\s*(\S+)/gc) {
          my $v = $1;
          last if $v eq '$;' || $v eq '$.' || $v eq '$. ';
          $isvar{$v} = 1;
        }
        pos($s) = pos($s);
        $mode = 'idle';
      }
      next;
    }
    if ($t eq '$d') { $mode = 'skip_semi'; next }
    if ($t eq '$f') { $mode = 'f'; next }
    if ($t eq '$e') { $mode = 'e'; next }
    if ($t eq '$a') { $mode = 'a'; next }
    if ($t eq '$p') { $mode = 'p_concl'; next }

    $pending = $t;
  }

  # Parsing needs the syntax axioms: without them `-.` swallows its argument
  # (so `wn` looks like a 0-argument constructor) and `|- A = C` parses as the
  # bare atom `A`, losing the rest.
  #
  # Each syntax axiom is recorded with its own typecode and its surface token
  # list.  Tokens that are `$v` variables are *slots*; every slot's typecode
  # comes from the `$f` declaration of that variable (`A.` takes a `setvar`
  # then a `wff`, so the `x` in `A. x ph` can never be parsed as `x e. A`).
  # That is the whole of Metamath's grammar, and it makes the parse both
  # deterministic and constructor-named: the node for `( ph -> ps )` is
  # `['wi', ph, ps]`, the same shape `set.mm0` writes.
  my (%vartype, %synax);
  # `$f` declarations give every variable its typecode; `$f` statements are not
  # in @order (only assertions are), so scan the whole statement table.
  for my $l (keys %st) {
    next unless $st{$l}{type} eq 'f';
    my @tok = split ' ', $st{$l}{concl};
    $vartype{ $tok[-1] } = $tok[0] if @tok;
  }
  for my $l (@order) {
    my $d = $st{$l};
    my @tok = split ' ', $d->{concl};
    next unless @tok;
    next unless $d->{type} eq 'a';
    next unless @tok >= 2;
    next if $tok[0] eq '|-';            # a logical axiom, not syntax
    push @{ $synax{ $tok[0] } }, { label => $l, tok => [ @tok[ 1 .. $#tok ] ] };
  }

  my %ord; $ord{$order[$_]} = $_ for 0 .. $#order;
  return { st => \%st, order => \@order, ord => \%ord, isvar => \%isvar,
           vartype => \%vartype, synax => \%synax,
           cache => { form => {}, req => {}, hyp => {}, refs => {} } };
}

# ---------------------------------------------------------------- decoding
# Split a proof section into its `( hints )` list and the letter blob.
sub split_proof {
  my ($proof) = @_;
  my @tok = split ' ', $proof;
  my (@hints, $i);
  if (@tok && $tok[0] eq '(') {
    $i = 1;
    while ($i < @tok && $tok[$i] ne ')') { push @hints, $tok[$i]; $i++ }
    $i++;                                   # skip ')'
  } else {
    $i = 0;
  }
  my $letters = join('', @tok[$i .. $#tok]);
  return (\@hints, $letters);
}

# Decode a letter blob into references.  Each reference is [kind, value]:
#   ['ref', n]      n-th entry of the label map
#   ['local', n]    share step n (0-based) of this very proof
#   ['end']         no such thing; used only as a terminator
sub decode_letters {
  my ($letters) = @_;
  my @refs;
  my $val = 0;
  my $started = 0;
  for my $c (split //, $letters) {
    if ($c ge 'U' && $c le 'Y') {
      $val = $val * 5 + (ord($c) - ord('U') + 1);
      $started = 1;
    } elsif ($c ge 'A' && $c le 'T') {
      $val = $val * 20 + (ord($c) - ord('A'));
      push @refs, ['ref', $val];
      $val = 0; $started = 0;
    } elsif ($c eq 'Z') {
      push @refs, ['local', scalar(@refs)];
    } else {
      die "illegal character '$c' in compressed proof\n" if $c =~ /\S/;
    }
  }
  return \@refs;
}

# ---------------------------------------------------------------- simulation
# Rebuild the RPN structure of a proof.  Returns undef on a stack error, else
# { top => step, steps => [step...] } where a step is
#   { label => ..., args => [step...] }  for an assertion   (args: the $e ones)
#   { hyp => label }                     for a hypothesis
#   { share => step }                    for a local label reference
# Candidate label maps.  mmpars.c documents `hyps + hints`; the older book
# format is `hyps + every assertion declared earlier`.  The set.mm snapshot in
# this repo matches neither, so the construction has to be identified.
sub build_map {
  my ($db, $label, $mode) = @_;
  my $d = $db->{st}{$label};
  my ($hints) = split_proof($d->{proof});
  my @allh = map { [$_->[0], $_->[1]] } @{ $d->{hyps} };
  my @eh   = grep { $_->[1] eq 'e' } @allh;
  my @hints = map { [$_, 'a'] } @$hints;
  my $i = $db->{ord}{$label};
  my @before = $i ? map { [$_, 'a'] } @{ $db->{order} }[0 .. $i - 1] : ();
  my %m = (
    hyps_hints        => [@allh, @hints],
    ehyps_hints       => [@eh,   @hints],
    hints_hyps        => [@hints, @allh],
    hints             => [@hints],
    before_hints      => [@before, @hints],
    hints_before      => [@hints, @before],
    before            => [@before],
    hyps_before       => [@allh, @before],
    ehyps_before      => [@eh,   @before],
    hyps_hints_before => [@allh, @hints, @before],
  );
  return $m{$mode};
}

sub simulate {
  my ($db, $label, %opt) = @_;
  my $st = $db->{st};
  my $d  = $st->{$label} or return undef;
  return undef unless $d->{type} eq 'p';
  my ($hints, $letters) = split_proof($d->{proof});
  my @refs = @{ decode_letters($letters) };
  my $mapref = $opt{map_ref} || build_map($db, $label, $opt{map} || 'hyps_hints');
  return undef unless $mapref;
  my @map = @$mapref;
  my $pops = $opt{pops} || 'e';    # 'e' = only $e, 'all' = $f + $e

  my @stack;
  my @steps;
  my @local;      # Z-named expressions, in the order the Zs appear
  for my $r (@refs) {
    my ($kind, $v) = @$r;
    if ($kind eq 'local') {
      # Z names the expression the last step produced, and takes the next map
      # slot so a later reference can reuse it
      return undef unless @stack;
      $map[ scalar(@map) ] = [ '<local>', 'local', scalar(@local) ];
      push @local, $stack[-1];
      next;
    }
    return undef if $v > $#map;
    my ($name, $type, $idx) = @{ $map[$v] };
    if ($type eq 'local') { push @stack, { share => $local[$idx] }; next }
    if ($type eq 'f' || $type eq 'e') {
      push @stack, { hyp => $name };
      next;
    }
    my $s = $st->{$name} or return undef;
    return undef unless $s->{type} eq 'a' || $s->{type} eq 'p';
    my @hy = @{ $s->{hyps} };
    my $need = $pops eq 'all' ? scalar(@hy) : scalar(grep { $_->[1] eq 'e' } @hy);
    return undef if @stack < $need;
    my @arg = splice(@stack, -$need);
    my @eargs;
    if ($pops eq 'all') {
      my $k = 0;
      for my $h (@hy) {
        my $a = $arg[$k++];
        push @eargs, $a if $h->[1] eq 'e';
      }
    } else {
      @eargs = @arg;
    }
    push @stack, { label => $name, args => \@eargs };
  }
  return undef unless @stack == 1;
  return { top => $stack[0], steps => \@steps };
}

# ---------------------------------------------------------------- derivation
# A decoded proof is only worth having if it actually checks.  The routines
# below re-run the RPN stream with real unification: every assertion's
# hypotheses are matched against the stack entries it is applied to, its
# conclusion is instantiated from the resulting substitution, and the final
# stack entry must unify with the theorem's own conclusion.  A stream that
# passes that is a derivation, not a guess.
#
# The label map itself is measured, not assumed.  `metamath-exe`'s decompressor
# on this snapshot shows the prefix is *not* `reqHypList` (13 entries for `a1i`,
# nearly all unused):
#
#   a1i   ABADCABEF        0,1,0,3,2,0,1,4,5   wph wps a1i.1 | wi ax-1 ax-mp
#   mpbir BACABDEF        1,0,2,0,1,3,4,5     wph wps mpbir.min mpbir.maj | biimpri ax-mp
#   id    AAABZAAACAECD    0,0,0,1,(Z),0,0,0,2,0,4,2,3
#                                            wph | wi ax-1 mpd  (+ one Z slot)
#
# i.e. the `$f` hypotheses of the variables that occur, then the `$e`
# hypotheses that the proof uses, then the hint list; `Z` appends one slot that
# a later reference reuses.  Which `$e` hypotheses are used is not known before
# decoding, so the candidates enumerate them and the unifier picks the winner.

sub _form_tree {
  my ($db, $f) = @_;
  return undef unless defined $f;
  return $db->{cache}{form}{$f} if exists $db->{cache}{form}{$f};
  my $tree = _form_tree_raw($db, $f);
  return $db->{cache}{form}{$f} = $tree;
}

sub _form_tree_raw {
  my ($db, $f) = @_;
  return undef unless defined $f;
  my @tok = split ' ', $f;
  my $typec = shift @tok;   # `|-`, `wff`, `class`, `setvar`
  # `|-` is the provable-formula typecode and is not a syntax typecode: what
  # follows it is a wff.
  $typec = 'wff' if $typec eq '|-';
  my $i = 0;
  my $isvar   = $db->{isvar};
  my $vartype = $db->{vartype} || {};
  my $synax   = $db->{synax} || {};

  # `_expr($typecode)` parses one expression of that typecode at $i.  A `$v`
  # variable of that type is an atom; otherwise some syntax axiom of that type
  # must match, its literal tokens compared directly and its variable slots
  # parsed with *their* typecodes.  When a slot's typecode does not admit the
  # tokens at hand the whole axiom is abandoned and the next is tried.
  my $depth = 0;
  my $expr; $expr = sub {
    my ($want) = @_;
    my $t = $tok[$i];
    return undef unless defined $t;
    return undef if $depth > 200;
    if ($isvar->{$t} && ($vartype->{$t} || '') eq $want) { $i++; return $t }
    $depth++;
    for my $ax (@{ $synax->{$want} || [] }) {
      my $save = $i;
      my @args;
      my $ok = 1;
      for my $p (@{ $ax->{tok} }) {
        if ($isvar->{$p}) {
          my $sub = $expr->($vartype->{$p} || '');
          if (!defined $sub) { $ok = 0; last }
          push @args, $sub;
        } else {
          if (!defined $tok[$i] || $tok[$i] ne $p) { $ok = 0; last }
          $i++;
        }
      }
      if ($ok) { $depth--; return [ $ax->{label}, @args ] }
      $i = $save;
    }
    $depth--;
    return undef;
  };

  my $tree = $expr->($typec);
  return undef unless defined $tree;
  return undef unless $i == @tok;   # the whole formula must be consumed
  return $tree;
}

# The expression a hypothesis contributes is its own formula -- except for a
# `$f wff ph`, whose expression is just the variable `ph` (a wff expression),
# not `( wff ph )`.
sub _hyp_tree {
  my ($db, $name, $type) = @_;
  my $key = "$type/$name";
  return $db->{cache}{hyp}{$key} if exists $db->{cache}{hyp}{$key};
  my $f = $db->{st}{$name}{concl};
  return undef unless defined $f;
  if ($type eq 'f') {
    my @t = split ' ', $f;
    return $db->{cache}{hyp}{$key} = $t[-1];
  }
  return $db->{cache}{hyp}{$key} = _form_tree($db, $f);
}

sub _dump_tree {
  my ($n) = @_;
  return ref $n ? '(' . join(' ', map { _dump_tree($_) } @$n) . ')' : $n;
}

# Resolve a tree through the substitution, ONE level only: a variable's value
# is written once and is never re-substituted.  This is the crux of Metamath's
# `$f`-hypothesis semantics.  A stack entry is a *ground* expression over the
# enclosing theorem's variables, so when a step binds `ps := ( ph -> ps )` the
# `ps` inside that value is the object variable, not this step's metavariable
# still to be solved.  Resolving values (or resolving the expression side while
# unifying) produces the spurious cycle `ph := ps`, `ps := ps` and is wrong.
sub _res {
  my ($db, $n, $subst) = @_;
  return $n if ref $n;
  return $n unless $db->{isvar}{$n};
  return exists $subst->{$n} ? $subst->{$n} : $n;
}

# unify a template (with Metamath variables) against a concrete expression
sub _unify {
  my ($db, $pat, $expr, $subst) = @_;
  return 0 unless defined $pat && defined $expr;
  if (!ref $pat) {
    if ($db->{isvar}{$pat}) {
      if (exists $subst->{$pat}) { return _tree_eq($subst->{$pat}, $expr) }
      $subst->{$pat} = $expr;
      return 1;
    }
    return 0 if ref $expr;
    return $pat eq $expr;
  }
  return 0 unless ref $expr && @$pat == @$expr;
  for my $k (0 .. $#$pat) { return 0 unless _unify($db, $pat->[$k], $expr->[$k], $subst) }
  return 1;
}

# Structural equality.  Comparing a bound variable's value by dumping both trees
# to strings allocates a whole expression per comparison and makes long proofs
# quadratic -- that is what hung `decode-check.pl` around statement 40000.
sub _tree_eq {
  my ($a, $b) = @_;
  return 0 unless defined $a && defined $b;
  my ($ra, $rb) = (ref $a, ref $b);
  return 0 if (defined $ra) != (defined $rb);
  return $a eq $b unless $ra;
  return 0 unless @$a == @$b;
  for my $k (0 .. $#$a) { return 0 unless _tree_eq($a->[$k], $b->[$k]) }
  return 1;
}

sub _inst {
  my ($db, $pat, $subst) = @_;
  return _res($db, $pat, $subst) unless ref $pat;
  return [ map { _inst($db, $_, $subst) } @$pat ];
}

# The hypotheses a step actually consumes.  A statement's declared hypothesis
# list in this file carries all twelve `$f wff ...` declarations, but a step
# only pops the `$f` hypotheses whose variable occurs in its own formula, plus
# every `$e` hypothesis.  `a1i`'s nine steps only balance under that rule
# (wi:2 + ax-1:2 + ax-mp:4 pops), and `metamath` verifies it.
sub req_hyps {  # public: hypothesis list a step actually pops
  my ($db, $label) = @_;
  return _req_hyps($db, $label);
}

sub _req_hyps {
  my ($db, $label) = @_;
  return $db->{cache}{req}{$label} if $db->{cache}{req}{$label};
  my $d  = $db->{st}{$label};
  my $st = $db->{st};
  my %occ;
  my $scan; $scan = sub {
    my ($n) = @_;
    return unless defined $n;
    if (!ref $n) { $occ{$n} = 1 if $db->{isvar}{$n}; return }
    $scan->($_) for @$n;
  };
  $scan->(_form_tree($db, $d->{concl}));
  my @all = @{ $d->{hyps} };
  for my $h (grep { $_->[1] eq 'e' } @all) {
    $scan->(_hyp_tree($db, $h->[0], $h->[1]));
  }
  my @f = grep {
    $_->[1] eq 'f' && $occ{ (split ' ', $st->{ $_->[0] }{concl})[-1] }
  } @all;
  my @e = grep { $_->[1] eq 'e' } @all;
  return $db->{cache}{req}{$label} = [@f, @e];
}

# `$f` hypotheses of the variables occurring in the theorem's statement
sub _occ_f {
  my ($db, $label) = @_;
  my $d  = $db->{st}{$label};
  my $st = $db->{st};
  my %occ;
  my $scan; $scan = sub {
    my ($n) = @_;
    return unless defined $n;
    if (!ref $n) { $occ{$n} = 1 if $db->{isvar}{$n}; return }
    $scan->($_) for @$n;
  };
  $scan->(_form_tree($db, $d->{concl}));
  for my $h (grep { $_->[1] eq 'e' } @{ $d->{hyps} }) {
    $scan->(_hyp_tree($db, $h->[0], $h->[1]));
  }
  return [ grep { $_->[1] eq 'f' && $occ{ (split ' ', $st->{ $_->[0] }{concl})[-1] } } @{ $d->{hyps} } ];
}

# candidate label maps, most likely first
sub candidate_maps {
  my ($db, $label) = @_;
  my $d = $db->{st}{$label};
  my $hints = _proof_refs($db, $label)->{hints};
  my @hints = map { my $t = $db->{st}{$_}{type}; [$_, $t eq 'a' || $t eq 'p' ? 'a' : $t] } @$hints;
  my @allf  = grep { $_->[1] eq 'f' } @{ $d->{hyps} };
  my @alle  = grep { $_->[1] eq 'e' } @{ $d->{hyps} };
  my @occf  = @{ _occ_f($db, $label) };
  # The proof prefix lists the mandatory hypotheses in *declaration* order, and
  # set.mm does interleave `$e` with `$f` (a hypothesis may use a variable that
  # is declared after it), so `$f`-then-`$e` is wrong: build one ordered list of
  # the mandatory hyps -- every `$e`, plus the `$f` of occurring variables.
  my %occvar = map { (split ' ', $db->{st}{ $_->[0] }{concl})[-1] => 1 } @occf;
  my (@base, %eidx);
  my $ei = 0;
  for my $h (@{ $d->{hyps} }) {
    if ($h->[1] eq 'e') { $eidx{ $h->[0] } = $ei++; push @base, $h }
    elsif ($h->[1] eq 'f') {
      my $v = (split ' ', $db->{st}{ $h->[0] }{concl})[-1];
      push @base, $h if !@occf || $occvar{$v};
    }
  }
  # prefix for a subset of the `$e` hypotheses, still in declaration order
  my $pre = sub {
    my ($mask) = @_;
    return [ map { $_->[1] eq 'e' && defined $mask && !($mask & (1 << $eidx{ $_->[0] }))
                   ? () : $_ } @base ];
  };
  my @fpre = @{ $pre->(undef) };
  # A hint is not necessarily an assertion: proofs routinely name a `$f` or
  # `$e` hypothesis in the hint list (dfbi1ALT cites the dummy variables `wch`,
  # `wth`), and such a step pushes that hypothesis' expression instead of
  # applying anything.  The type must come from the statement itself.
  my @out;
  my $n = @alle;
  if ($n <= 4) {
    for my $mask (reverse 0 .. (1 << $n) - 1) {          # all subsets, biggest first
      push @out, { name => 'occ+esub' . $mask, map => [@{ $pre->($mask) }, @hints] };
    }
  } else {
    push @out, { name => 'occ+e', map => [@fpre, @hints] };
    for my $skip (0 .. $#alle) {
      push @out, { name => 'occ+e_skip' . $skip, map => [@{ $pre->((1 << $n) - 1 - (1 << $skip)) }, @hints] };
    }
  }
  push @out, { name => 'occ',  map => [@fpre, @hints] };
  push @out, { name => 'all',  map => [@allf, @alle, @hints] };
  return @out;
}

# Decoding a proof is pure but expensive, and every candidate map re-runs the
# same stream; keep it.
sub _proof_refs {
  my ($db, $label) = @_;
  return $db->{cache}{refs}{$label} if $db->{cache}{refs}{$label};
  my $d = $db->{st}{$label};
  my ($hints, $letters) = split_proof($d->{proof} // '');
  my $refs = decode_letters($letters // '');
  return $db->{cache}{refs}{$label} = { hints => $hints, refs => $refs };
}

# run one candidate map; returns a verified derivation or undef
sub _run_map {
  my ($db, $label, $map) = @_;
  my $st = $db->{st};
  my $d  = $st->{$label};
  my @refs = @{ _proof_refs($db, $label)->{refs} };
  my @map = @$map;
  my @stack;
  my @ts;      # the same stack, as proof trees: leaf = hypothesis label
  my @local;
  my @tl;      # trees saved by `Z`, by local index
  my @steps;   # the reconstructed normal proof: labels in application order
  my $si = 0;
  for my $r (@refs) {
    my ($kind, $v) = @$r;
    warn sprintf("step %d (%s %s)\n", $si++, $kind, $v) if $ENV{MM_TRACE_STEPS};
    if ($kind eq 'local') {
      return undef unless @stack;
      $map[ scalar(@map) ] = ['<local>', 'local', scalar(@local)];
      push @local, $stack[-1];
      push @tl, $ts[-1];
      next;
    }
    if ($v > $#map) {
      warn "  ! ref $v beyond map ($#map slots)\n" if $ENV{MM_TRACE};
      return undef;
    }
    my ($name, $type, $idx) = @{ $map[$v] };
    if ($type eq 'local') { push @stack, $local[$idx]; push @ts, $tl[$idx]; next }
    if ($type eq 'f' || $type eq 'e') {
      my $t = _hyp_tree($db, $name, $type);
      return undef unless defined $t;
      push @steps, $name;
      push @stack, $t;
      push @ts, $name;
      next;
    }
    my $s = $st->{$name} or do { warn "  ! no statement $name\n" if $ENV{MM_TRACE}; return undef };
    unless ($s->{type} eq 'a' || $s->{type} eq 'p') {
      warn "  ! $name is type $s->{type}, not an assertion\n" if $ENV{MM_TRACE};
      return undef;
    }
    push @steps, $name;
    my @need = @{ _req_hyps($db, $name) };
    if (@stack < @need) {
      warn "  ! $name needs @{[scalar @need]} but stack has ", scalar(@stack), "\n" if $ENV{MM_TRACE};
      return undef;
    }
    # NB `splice(@stack, -0)` means `splice(@stack, 0)` and would wipe the
    # whole stack: constants like `cdr`/`cmgp` take no arguments at all.
    my @arg = @need ? splice(@stack, -@need) : ();
    my @targ = @need ? splice(@ts, -@need) : ();
    my %subst;
    for my $k (0 .. $#need) {
      my $hpat = _hyp_tree($db, $need[$k][0], $need[$k][1]);
      unless (_unify($db, $hpat, $arg[$k], \%subst)) {
        warn "  ! $name hyp $need[$k][0] pat ", _dump_tree($hpat),
             " vs arg ", _dump_tree($arg[$k]), "\n" if $ENV{MM_TRACE};
        return undef;
      }
    }
    my $concl = _inst($db, _form_tree($db, $s->{concl}), \%subst);
    return undef unless defined $concl;
    warn sprintf("  %-10s pop %d -> %s (sp=%d)\n", $name, scalar(@need),
                 _dump_tree($concl), scalar(@stack) + 1) if $ENV{MM_TRACE};
    push @stack, $concl;
    # The step's instantiated conclusion is kept beside its arguments: a
    # dependent-hypothesis axiom (set.mm0 spells `ax_gen` that way) has a
    # Π-bound parameter among its hypotheses, and that argument is fixed by the
    # conclusion rather than by anything the proof stream pushes.
    push @ts, { name => $name, args => \@targ, res => $concl };
  }
  return undef unless @stack == 1;
  my %subst;
  my $goal = _form_tree($db, $d->{concl});
  unless (defined $goal && _unify($db, $goal, $stack[0], \%subst)) {
    warn "  ! goal ", (defined $goal ? _dump_tree($goal) : 'UNDEF'),
         " vs top ", _dump_tree($stack[0]), "\n" if $ENV{MM_TRACE};
    return undef;
  }
  return { tree => $stack[0], proof_tree => $ts[0], map => $map, steps => \@steps };
}

# the public entry point: decode this theorem's proof into a checked derivation
sub derive {
  my ($db, $label) = @_;
  my $d = $db->{st}{$label} or return undef;
  return undef unless $d->{type} eq 'p';
  for my $cm (candidate_maps($db, $label)) {
    my $r = _run_map($db, $label, $cm->{map});
    if ($r) { $r->{mode} = $cm->{name}; return $r }
  }
  return undef;
}

# ---------------------------------------------------------------- shapes
# canonical shape of a formula: variables (Metamath $v names) become #k in
# order of first occurrence, everything else stays.  Strings without outer
# parentheses are wrapped first, since Metamath writes applications that way.
sub shape {
  my ($db, $formula) = @_;
  my $isvar = $db->{isvar};
  my @tok = split ' ', $formula;
  my $i = 0;
  my %seen;
  my $node;
  $node = sub {
    my $t = $tok[$i++];
    die "unbalanced formula\n" unless defined $t;
    if ($t eq '(' ) {
      my @args;
      while (defined $tok[$i] && $tok[$i] ne ')') { push @args, $node->() }
      die "unbalanced formula\n" unless defined $tok[$i];
      $i++;
      return @args == 1 ? $args[0] : \@args;
    }
    return $t if $t eq ')';
    return $isvar->{$t} ? do {
      $seen{$t} = scalar(keys %seen) + 1 unless exists $seen{$t};
      "#$seen{$t}";
    } : $t;
  };
  my $r = $node->();
  my $dump;
  $dump = sub {
    my $n = shift;
    return ref $n ? "(" . join(" ", map { $dump->($_) } @$n) . ")" : $n;
  };
  return $dump->($r);
}

1;
