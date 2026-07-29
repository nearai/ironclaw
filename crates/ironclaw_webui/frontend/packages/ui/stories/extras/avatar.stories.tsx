import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Avatar, AvatarFallback, AvatarImage } from "../../src/extras/avatar";

const meta: Meta = { title: "Extras/Avatar" };
export default meta;

type Story = StoryObj;

export const WithImage: Story = {
  render: () => (
    <Avatar>
      <AvatarImage
        src="https://avatars.githubusercontent.com/u/9919?v=4"
        alt="GitHub avatar"
      />
      <AvatarFallback>GH</AvatarFallback>
    </Avatar>
  ),
};

export const FallbackOnly: Story = {
  render: () => (
    <Avatar>
      <AvatarFallback>AL</AvatarFallback>
    </Avatar>
  ),
};

export const Sizes: Story = {
  render: () => (
    <div className="flex items-center gap-3">
      <Avatar size="sm"><AvatarFallback>S</AvatarFallback></Avatar>
      <Avatar size="md"><AvatarFallback>M</AvatarFallback></Avatar>
      <Avatar size="lg"><AvatarFallback>L</AvatarFallback></Avatar>
    </div>
  ),
};
