package com.geek.auth.model;

public class User {
    private String username;
    private String passwordMd5;

    public String getUsername() { return username; }
    public void setUsername(String username) { this.username = username; }
    public String getPasswordMd5() { return passwordMd5; }
    public void setPasswordMd5(String passwordMd5) { this.passwordMd5 = passwordMd5; }
}
