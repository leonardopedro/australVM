#!/usr/bin/perl
# Decode set.mm's compressed proofs into checked derivations.
#
#   decode-check.pl [N] [--spec PATH] [--all] [--proof LABEL] [--quiet]
#
# Each proof is decoded and then *checked* the way the MM0 compiler checks a
# proof: every step pops the hypotheses it needs, the popped expressions are
# unified against those hypotheses, the conclusion is instantiated, and the
# final stack entry must unify with the theorem's own statement.  A proof that
# survives that is a derivation, not a guess -- no external Metamath binary is
# consulted.
#
# Exit status: 0 if every sampled proof decoded, 1 otherwise.
use strict;
use warnings;
use FindBin;
use lib $FindBin::Bin;
use SetMM;

my $spec  = "$FindBin::Bin/../metamath/set.mm";
my $limit = 0;          # 0 = all
my $quiet = 0;
my $from  = 1;          # 1-based index into the assertion order
my $to    = 0;          # 0 = no bound
my @proofs;
my @argv = @ARGV;
while (@argv) {
  my $a = shift @argv;
  if    ($a eq '--spec')  { $spec = shift @argv }
  elsif ($a eq '--proof') { push @proofs, shift @argv }
  elsif ($a eq '--all')   { $limit = 0 }
  elsif ($a eq '--quiet') { $quiet = 1 }
  elsif ($a eq '--from')  { $from = shift @argv }
  elsif ($a eq '--to')    { $to = shift @argv }
  elsif ($a =~ /^\d+$/)   { $limit = $a }
  else { die "usage: $0 [N] [--spec PATH] [--proof LABEL] [--from I --to J] [--quiet]\n" }
}

my $t0 = time;
my $db = SetMM::parse($spec);
printf "parsed %d statements in %ds\n", scalar(keys %{ $db->{st} }), time - $t0
  unless $quiet;

if (@proofs) {
  for my $l (@proofs) {
    my $r = SetMM::derive($db, $l);
    if (!$r) { print "$l: FAIL\n"; next }
    my $goal = SetMM::_dump_tree(SetMM::_form_tree($db, $db->{st}{$l}{concl}));
    printf "%s: OK (%s, %d steps)\n  goal %s\n  %s\n", $l, $r->{mode},
      scalar(@{ $r->{steps} }), $goal, join(' ', @{ $r->{steps} });
  }
  exit 0;
}

my ($ok, $bad) = (0, 0);
my (@fail, %mode);
my $n = 0;
for my $l (@{ $db->{order} }) {
  next unless $db->{st}{$l}{type} eq 'p';
  $n++;
  last if $limit && $n > $limit;
  next if $n < $from;
  last if $to && $n > $to;
  my $r = SetMM::derive($db, $l);
  if ($r) { $ok++; $mode{ $r->{mode} }++ }
  else    { $bad++; push @fail, $l if @fail < 20 }
  printf "  ... %d (%d ok, %d bad)\n", $n, $ok, $bad
    if !$quiet && $n % 5000 == 0;
}
printf "range=%d..%d ok=%d bad=%d in %ds\n", $from, $to, $ok, $bad, time - $t0;
printf "  mode %-12s => %d\n", $_, $mode{$_} for sort { $mode{$b} <=> $mode{$a} } keys %mode;
print "  FAIL: @fail\n" if @fail;
exit($bad ? 1 : 0);
