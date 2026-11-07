#!/usr/bin/perl
# shapegrep.pl -- find set.mm (as MM0) declarations by statement shape.
#
# Porting a Lean proof onto set.mm by hand means reading labels until you find
# the theorem whose *conclusion* fits the step you need.  This script does that
# lookup: give it a statement shape (MM0 prefix form, with `?` for "anything"
# and free names for "the same thing everywhere"), and it reports every
# declaration whose conclusion matches, the instantiation it found, and the
# hypotheses you would still owe.
#
#   perl shapegrep.pl '(wceq (csu x (csn A) B) C)'
#   perl shapegrep.pl -s '(csu x (csn A) B)'      # pattern anywhere in concl
#   perl shapegrep.pl --name '^ind'               # by label
#   perl shapegrep.pl -f '(wi (wral x P A) (wi (wcel (cv x) A) P))'
#
# Options:
#   -s, --subterm     match the pattern anywhere inside the conclusion
#   -f, --full        print the hypotheses of each match
#   -v, --verbose     print the variable instantiation found
#   -n N              limit results (default 20, 0 = no limit)
#   -k KIND           restrict to theorem|axiom|term|def
#       --name RE     filter declarations whose label matches RE
#   -q                label only
#   --db PATH         database (default: ../metamath/set.mm0 next to this script)
#
# Exit status: 0 if something matched, 1 if nothing did, 2 on a bad pattern.

use strict;
use warnings;

my ($pat, $mode, $full, $verbose, $limit, $kind, $name_re, $quiet) =
  (undef, 'concl', 0, 0, 20, undef, undef, 0);

my @argv = @ARGV;
while (@argv) {
  my $a = shift @argv;
  if    ($a eq '-s' || $a eq '--subterm') { $mode = 'subterm' }
  elsif ($a eq '-f' || $a eq '--full')    { $full = 1 }
  elsif ($a eq '-v' || $a eq '--verbose') { $verbose = 1 }
  elsif ($a eq '-q')                      { $quiet = 1 }
  elsif ($a eq '-n')                      { $limit = shift @argv }
  elsif ($a eq '-k')                      { $kind = shift @argv }
  elsif ($a eq '--name')                  { $name_re = shift @argv }
  elsif ($a eq '--db')                    { $ENV{SETMM0} = shift @argv }
  elsif ($a =~ /^-/)                      { die "shapegrep: unknown option $a\n" }
  else                                    { push @argv, $a; last }
}
$pat = shift @argv if @argv;

unless (defined $name_re || defined $pat) {
  print STDERR "usage: shapegrep.pl [options] '(<pattern>)'\n";
  exit 2;
}

my $db = $ENV{SETMM0};
unless (defined $db) {
  my $self = $0;
  $self =~ s{/[^/]*$}{};
  $db = ($self eq $0 ? '.' : $self) . '/../metamath/set.mm0';
}
open my $fh, '<', $db or die "shapegrep: cannot open $db: $!\n";

# ---------------------------------------------------------------- parsing ----
# A declaration runs from its keyword line to the next declaration keyword; the
# text between is a sequence of `$ ... $` expressions, the last being the
# conclusion (theorems/axioms) or a sort (terms/defs).

my @decls;
my $cur;
while (my $line = <$fh>) {
  next if $line =~ /^\s*--/;            # MM0 comment line
  if ($line =~ /^(theorem|axiom|term|def)\s+([^\s(:]+)/) {
    my ($k, $n) = ($1, $2);
    $line =~ s/^(theorem|axiom|term|def)\s+[^\s(:]+//;
    $cur = { kind => $k, name => $n, text => $line };
    push @decls, $cur;
    next;
  }
  next unless $cur;
  $cur->{text} .= $line;
}
close $fh;

sub parse_sexp {
  my ($s) = @_;
  my @tok = $s =~ /[()]|[^\s()]+/g;
  my $i = 0;
  my $sub;
  $sub = sub {
    my $t = $tok[$i++];
    return undef unless defined $t;
    if ($t eq '(') {
      my @a;
      while (defined $tok[$i] && $tok[$i] ne ')') { push @a, $sub->() }
      $i++ if defined $tok[$i];
      return ['', @a];                   # head is a constant, kept at [1]
    }
    return $t;
  };
  # A statement is a prefix application *without* an enclosing paren pair
  # (`wi (wcel A B) (wcel C D)`), so the first item is the head constant.
  my $head = $sub->();
  return $head unless defined $head && $i < @tok;
  my @a = ($head);
  push @a, $sub->() while $i < @tok;
  return ['', @a];
}

sub render {
  my ($e) = @_;
  return ref $e ? '(' . join(' ', map { render($_) } @$e[1 .. $#$e]) . ')' : $e;
}

for my $d (@decls) {
  my $t = $d->{text};
  my $pre = $t;
  $pre =~ s/\$.*//s;                    # binders only
  my %vars;
  while ($pre =~ /\{([^}]*)\}|\(([^)]*)\)/g) {
    my $g = defined $1 ? $1 : $2;
    next unless $g =~ /:/;
    my ($names, $sort) = split /:/, $g, 2;
    my ($s) = split ' ', $sort;
    $vars{$_} = $s for grep { length } split ' ', $names;
  }
  $d->{vars} = \%vars;

  my @f = ($t =~ /\$([^\x24]*)\$/g);
  if (@f) {
    $d->{concl} = parse_sexp($f[-1]);
    $d->{hyps}  = [ map { parse_sexp($_) } @f[0 .. $#f - 1] ];
  }
  else {                                # term/def: `: sort;`
    $d->{concl} = undef;
    ($d->{sort}) = ($t =~ /:\s*(\w+)\s*;/);
  }
}

# --------------------------------------------------------------- matching ----
# `?` matches any subterm.  Any name mentioned in the pattern that is not `?`
# is a pattern variable and must be instantiated consistently; a name that is a
# set.mm constant (never a pattern variable, since constants are not declared in
# the pattern's binder list) matches only itself.

my %const = map { $_->{name} => 1 } grep { $_->{kind} eq 'term' || $_->{kind} eq 'def' } @decls;

my ($pats, $pats_vars);
if (defined $pat) {
  my $s = $pat;
  $s =~ s/^\s*\$//; $s =~ s/\$\s*$//;
  my $tree = parse_sexp($s);
  unless (defined $tree) { print STDERR "shapegrep: cannot parse pattern\n"; exit 2 }

  # Pattern variables: every atom that is not a constant and not `?`/number.
  my %pv;
  my $scan; $scan = sub {
    my ($e) = @_;
    if (ref $e) { $scan->($_) for @$e[1 .. $#$e]; return }
    return if $e eq '?' || $const{$e} || $e =~ /^-?\d+$/;
    $pv{$e} = 1;
  };
  $scan->($tree);
  $pats = $tree; $pats_vars = \%pv;
}

sub atom_equal {
  my ($a, $b) = @_;
  return render($a) eq render($b);
}

sub try_match {
  my ($p, $t, $b) = @_;
  if (!ref $p) {
    return 1 if $p eq '?';
    if ($pats_vars->{$p}) {
      if (exists $b->{$p}) { return atom_equal($b->{$p}, $t) ? 1 : 0 }
      $b->{$p} = $t;
      return 1;
    }
    return (!ref $t && $t eq $p) ? 1 : 0;
  }
  return 0 unless ref $t;
  return 0 unless @$p == @$t;
  for my $i (1 .. $#$p) {
    return 0 unless try_match($p->[$i], $t->[$i], $b);
  }
  return 1;
}

sub contains_match {
  my ($t, $b) = @_;
  return 1 if try_match($pats, $t, $b);
  return 0 unless ref $t;
  for my $i (1 .. $#$t) {
    my %b2 = %$b;
    return 1 if contains_match($t->[$i], \%b2);
  }
  return 0;
}

# ------------------------------------------------------------------ report ---
my $hits = 0;
for my $d (@decls) {
  next if defined $kind && $d->{kind} ne $kind;
  next if defined $name_re && $d->{name} !~ /$name_re/;
  my $ok = 0;
  my %b;
  if (defined $pat) {
    if ($mode eq 'subterm') { $ok = $d->{concl} && contains_match($d->{concl}, {%b}) }
    else                    { $ok = $d->{concl} && try_match($pats, $d->{concl}, \%b) }
  }
  else { $ok = 1 }
  next unless $ok;

  $hits++;
  last if $limit && $hits > $limit;

  if ($quiet) { print "$d->{name}\n"; next }

  my @hyps = @{ $d->{hyps} || [] };
  printf "%s  [%s%s]\n", $d->{name}, $d->{kind},
    (@hyps ? ", " . scalar(@hyps) . " hyp" . (@hyps > 1 ? "s" : "") : "");
  my $c = $d->{concl} ? render($d->{concl}) : ($d->{sort} // '');
  print "    $c\n";
  if ($full) {
    for my $i (0 .. $#hyps) { printf "    h%d: %s\n", $i + 1, render($hyps[$i]) }
  }
  if ($verbose) {
    for my $k (sort keys %b) {
      next if !ref $b{$k} && !defined $b{$k};
      printf "    %s := %s\n", $k, render($b{$k});
    }
  }
}

print STDERR "(no match)\n" if !$hits && !$quiet;
exit($hits ? 0 : 1);
