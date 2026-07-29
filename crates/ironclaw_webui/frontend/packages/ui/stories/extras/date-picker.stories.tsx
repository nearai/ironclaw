import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Calendar, DatePicker } from "../../src/extras/date-picker";

const meta: Meta = { title: "Extras/DatePicker" };
export default meta;

type Story = StoryObj;

function CalendarDemo(props: { minDate?: Date; maxDate?: Date; weekStartsOn?: 0 | 1 }) {
  const [value, setValue] = React.useState<Date | null>(new Date());
  return <Calendar value={value} onChange={setValue} {...props} />;
}

export const InlineCalendar: Story = { render: () => <CalendarDemo /> };

export const MondayFirstWithBounds: Story = {
  render: () => (
    <CalendarDemo
      weekStartsOn={1}
      minDate={new Date(new Date().getFullYear(), new Date().getMonth(), 5)}
      maxDate={new Date(new Date().getFullYear(), new Date().getMonth() + 1, 20)}
    />
  ),
};

function PickerDemo(props: { disabled?: boolean }) {
  const [value, setValue] = React.useState<Date | null>(null);
  return (
    <div className="h-96">
      <DatePicker
        value={value}
        onChange={setValue}
        aria-label="Due date"
        placeholder="Pick a due date"
        {...props}
      />
    </div>
  );
}

export const Picker: Story = { render: () => <PickerDemo /> };
export const PickerDisabled: Story = { render: () => <PickerDemo disabled /> };
