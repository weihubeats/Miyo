package com.example.calculator;

import java.util.ArrayList;
import java.util.List;

/**
 * 供 Miyo 调试验证的静态工具类：基本类型 / 字符串 / 集合 / 重载。
 * 零第三方依赖、零 Miyo 依赖——验收「源码零侵入」的基准目标。
 */
public final class Calculator {

    public static long add(long a, long b) {
        return a + b;
    }

    public static double divide(double a, double b) {
        if (b == 0) {
            throw new ArithmeticException("除数不能为 0");
        }
        return a / b;
    }

    public static String concat(String prefix, String suffix) {
        return prefix + "-" + suffix;
    }

    public static boolean isPalindrome(String input) {
        if (input == null) {
            return false;
        }
        String lowered = input.toLowerCase();
        int left = 0;
        int right = lowered.length() - 1;
        while (left < right) {
            if (lowered.charAt(left) != lowered.charAt(right)) {
                return false;
            }
            left++;
            right--;
        }
        return true;
    }

    public static List<String> fizzBuzz(int count) {
        List<String> out = new ArrayList<>();
        for (int i = 1; i <= count; i++) {
            if (i % 15 == 0) {
                out.add("FizzBuzz");
            } else if (i % 3 == 0) {
                out.add("Fizz");
            } else if (i % 5 == 0) {
                out.add("Buzz");
            } else {
                out.add(String.valueOf(i));
            }
        }
        return out;
    }

    /** 重载方法 A（两参）。 */
    public static String greet(String name) {
        return "Hello, " + name + "!";
    }

    /** 重载方法 B（三参）——重载定位验证用。 */
    public static String greet(String name, String greeting, boolean formal) {
        return (formal ? greeting : greeting.toLowerCase()) + ", " + name + "!";
    }

    private Calculator() {
    }
}
