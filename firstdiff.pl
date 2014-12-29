
$filename_a = "reftrace.txt";
$filename_b = "ztrace.txt";

open A, $filename_a || die "failed to open: $filename_a";
open B, $filename_b || die "failed to open: $filename_b";

my $a;
my $b;
my $line = 0;

@history = ();
push(@history, "foo");
push(@history, "bar");
print "history=@history\n";

$NHISTORY = 1000;

while (true) {    

    $a = <A>;
    $b = <B>;

    chop $a;
    chop $b;

    if ($a eq $b) {
        # print "equal: $a\n";

        $len = scalar @history;
        # print "length = ", $len, "\n";
        if ($len > $NHISTORY) {
            # print "shifting\n";
            shift(@history);
        }
        $x = push(@history, $a);
        # print "after push: $x @history\n";
    }
    else {
        print "===== DIVERGENCE at line $line =====\n";
        print "Shared history:\n\n";
        for $x (@history) {
            print "same   : $x\n";
        }

        # print "Divergent blocks:\n\n";
        print "\n";

        print "$filename_a:\n";
        print "A      : $a\n";
        $n = 1;
        while ($a = <A>) {
            chop $a;
            print "A      : ", $a, "\n";
            last if $n++ == 10;
        }

        print "\n";
        print "$filename_b:\n";
        print "B      : $b\n";
        $n = 1;
        while ($b = <B>) {
            chop $b;
            print "B      : ", $b, "\n";
            last if $n++ == 10;
        }

        last;
    }

    $line++;
}


print "lines: $line\n";
