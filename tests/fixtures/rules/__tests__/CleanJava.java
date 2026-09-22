package fixtures.rules;

import java.util.logging.Logger;

public class CleanJava {

    private static final Logger LOG = Logger.getLogger("clean");

    int classify(int x) {
        if (x > 0) {
            return 1;
        }
        return 0;
    }

    void safeCatch() {
        try {
            risky();
        } catch (Exception e) {
            LOG.warning("failed");
        }
    }

    void ignoredCatch() {
        try {
            risky();
        } catch (Exception e) {
            // explicitly ignored: best-effort cleanup
        }
    }

    int compute(int x) {
        if (x > 0) {
            LOG.info("positive");
        } else {
            return -1;
        }
        return 0;
    }

    void risky() {
    }
}
